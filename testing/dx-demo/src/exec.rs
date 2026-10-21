//! Real block execution with revm, plus the per-transaction cost model the simulator charges.
//!
//! Every block in a run is executed exactly once against an in-memory state, in chain order, so
//! gas and failures are real. The time charged to the simulated validators comes from a cost
//! model calibrated with the same engine, which keeps the two protocols' costs identical for
//! identical transactions and free of measurement noise.

use std::{sync::mpsc, thread, time::Instant};

use alloy_primitives::{keccak256, Address, Bytes, TxKind as EvmTxKind, B256, U256};
use alloy_rlp::Encodable;
use alloy_trie::{HashBuilder, Nibbles, TrieAccount, EMPTY_ROOT_HASH, KECCAK_EMPTY};
use revm::{
    bytecode::Bytecode,
    context::TxEnv,
    database::{CacheDB, EmptyDB},
    handler::{MainnetContext, MainnetEvm},
    state::AccountInfo,
    Context, ExecuteCommitEvm, ExecuteEvm, MainBuilder, MainContext,
};
use serde::Serialize;

use crate::{
    config::SimConfig,
    workload::{Tx, TxKind},
};

/// BSC mainnet chain id.
const CHAIN_ID: u64 = 56;
/// Selector of `transfer(address,uint256)`.
const TRANSFER_SELECTOR: [u8; 4] = [0xa9, 0x05, 0x9c, 0xbb];
/// Runtime code of the demo token. Balances live in the slot keyed by the holder address.
///
/// ```text
/// amt = calldata[0x24]; bal = sload(caller); if bal < amt revert
/// sstore(caller, bal - amt); to = calldata[0x04]; sstore(to, sload(to) + amt)
/// ```
const TOKEN_CODE: [u8; 32] = [
    0x60, 0x24, 0x35, 0x33, 0x54, 0x81, 0x81, 0x10, 0x60, 0x1a, 0x57, 0x81, 0x90, 0x03, 0x33, 0x55,
    0x60, 0x04, 0x35, 0x80, 0x54, 0x82, 0x01, 0x90, 0x55, 0x00, 0x5b, 0x60, 0x00, 0x80, 0xfd, 0x00,
];
/// Transactions per calibration block.
const CALIBRATION_BLOCK: usize = 2000;

/// Result of executing one block.
#[derive(Debug, Clone, Copy, Default)]
pub struct BlockExec {
    /// Gas used by all transactions.
    pub gas_used: u64,
    /// Transactions that failed validation or reverted.
    pub failed: usize,
    /// Native transfers in the block.
    pub transfers: usize,
    /// ERC20 transfers in the block.
    pub erc20: usize,
}

/// Per-transaction execution cost charged to the simulated validators.
#[derive(Debug, Clone, Serialize)]
pub struct ExecCostModel {
    /// Native transfer, including its share of the state root.
    pub transfer_us: f64,
    /// ERC20 transfer, including its share of the state root.
    pub erc20_us: f64,
    /// Fixed cost per block.
    pub block_us: f64,
    /// A block never takes less than this to execute (`--exec-block-ms`).
    pub min_block_us: f64,
    /// Cost measured with in-memory revm before scaling, for the report.
    pub measured_transfer_us: f64,
    /// Cost measured with in-memory revm before scaling, for the report.
    pub measured_erc20_us: f64,
}

impl ExecCostModel {
    /// Measures in-memory revm and scales it to `cfg.exec_us_per_tx` when that is set.
    pub fn calibrate(cfg: &SimConfig) -> Self {
        let accounts = cfg.accounts.max(4);
        let mut engine = ExecEngine::new(accounts);
        let mut nonces = vec![0u64; accounts];
        let mut measure = |kind| {
            let txs: Vec<Tx> = (0..CALIBRATION_BLOCK)
                .map(|i| {
                    let sender = i % accounts;
                    nonces[sender] += 1;
                    Tx {
                        sender: sender as u32,
                        recipient: ((i * 7 + 13) % accounts) as u32,
                        nonce: nonces[sender] - 1,
                        kind,
                        created_us: 0,
                    }
                })
                .collect();
            let start = Instant::now();
            let res = engine.execute(txs.iter());
            assert_eq!(res.failed, 0, "calibration transactions must succeed");
            start.elapsed().as_secs_f64() * 1e6 / CALIBRATION_BLOCK as f64
        };
        let mut transfer = f64::MAX;
        let mut erc20 = f64::MAX;
        for _ in 0..3 {
            transfer = transfer.min(measure(TxKind::Transfer));
            erc20 = erc20.min(measure(TxKind::Erc20));
        }
        let scale = cfg.exec_us_per_tx.map_or(1.0, |target| {
            let mean = transfer * (1.0 - cfg.erc20_share) + erc20 * cfg.erc20_share;
            target / mean
        });
        Self {
            transfer_us: transfer * scale,
            erc20_us: erc20 * scale,
            block_us: cfg.block_overhead_ms * 1000.0,
            min_block_us: cfg.exec_block_ms.unwrap_or(0.0) * 1000.0,
            measured_transfer_us: transfer,
            measured_erc20_us: erc20,
        }
    }

    /// Cost of executing a block with the given composition.
    pub fn block_cost(&self, exec: &BlockExec) -> f64 {
        let cost = self.block_us +
            exec.transfers as f64 * self.transfer_us +
            exec.erc20 as f64 * self.erc20_us;
        cost.max(self.min_block_us)
    }

    /// Cost of a block from its composition alone, for blocks that are never executed on the
    /// shared engine (orphaned or rejected ones).
    pub fn cost_of<'a>(&self, txs: impl Iterator<Item = &'a Tx>) -> f64 {
        let mut exec = BlockExec::default();
        for tx in txs {
            match tx.kind {
                TxKind::Transfer => exec.transfers += 1,
                TxKind::Erc20 => exec.erc20 += 1,
            }
        }
        self.block_cost(&exec)
    }
}

/// Executes canonical blocks on an [`ExecEngine`] in a background thread, so the real revm work
/// that makes gas and failures real does not stall a wall-clock event loop. Charged execution
/// time comes from [`ExecCostModel::cost_of`], which depends only on block composition.
#[derive(Debug)]
pub struct BackgroundExec {
    tx: mpsc::Sender<Vec<Tx>>,
    handle: thread::JoinHandle<usize>,
}

impl BackgroundExec {
    /// Starts the executor over `accounts` funded accounts.
    pub fn new(accounts: usize) -> Self {
        let (tx, rx) = mpsc::channel::<Vec<Tx>>();
        let handle = thread::spawn(move || {
            let mut engine = ExecEngine::new(accounts);
            rx.iter().map(|block| engine.execute(block.iter()).failed).sum()
        });
        Self { tx, handle }
    }

    /// Queues the next canonical block.
    pub fn submit<'a>(&self, txs: impl Iterator<Item = &'a Tx>) {
        self.tx.send(txs.cloned().collect()).expect("executor thread alive");
    }

    /// Waits for every queued block and returns the number of failed transactions.
    pub fn finish(self) -> usize {
        drop(self.tx);
        self.handle.join().expect("executor thread")
    }
}

/// In-memory EVM state shared by the run, executed block by block in chain order.
pub struct ExecEngine {
    evm: MainnetEvm<MainnetContext<CacheDB<EmptyDB>>>,
    token: Address,
}

impl std::fmt::Debug for ExecEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExecEngine").field("token", &self.token).finish_non_exhaustive()
    }
}

impl ExecEngine {
    /// Creates a genesis state with `accounts` funded accounts and the demo token.
    pub fn new(accounts: usize) -> Self {
        let mut db = CacheDB::new(EmptyDB::new());
        let balance = U256::from(10u128.pow(24));
        let token = Address::repeat_byte(0xee);
        db.insert_account_info(
            token,
            AccountInfo::from_bytecode(Bytecode::new_raw(Bytes::copy_from_slice(&TOKEN_CODE))),
        );
        for i in 0..accounts {
            let addr = account_address(i as u32);
            db.insert_account_info(addr, AccountInfo::from_balance(balance));
            db.insert_account_storage(token, address_slot(addr), balance)
                .expect("token account exists");
        }
        let evm = Context::mainnet()
            .with_db(db)
            .modify_cfg_chained(|cfg| cfg.chain_id = CHAIN_ID)
            .modify_block_chained(|block| {
                block.gas_limit = u64::MAX;
                block.basefee = 0;
            })
            .build_mainnet();
        Self { evm, token }
    }

    /// Executes `txs` in order, commits the state and computes a state root over the keys the
    /// block touched. The root is an approximation of an incremental trie update, whose cost
    /// grows with the number of touched keys rather than with the size of the state.
    pub fn execute<'a>(&mut self, txs: impl Iterator<Item = &'a Tx>) -> BlockExec {
        let mut out = BlockExec::default();
        let mut touched_accounts: Vec<Address> = Vec::new();
        let mut touched_slots: Vec<(Address, U256)> = Vec::new();
        for tx in txs {
            match tx.kind {
                TxKind::Transfer => out.transfers += 1,
                TxKind::Erc20 => out.erc20 += 1,
            }
            let caller = account_address(tx.sender);
            let recipient = account_address(tx.recipient);
            let (to, value, data, gas) = match tx.kind {
                TxKind::Transfer => (recipient, U256::from(1u64), Bytes::new(), tx.gas_limit()),
                TxKind::Erc20 => {
                    let mut data = Vec::with_capacity(68);
                    data.extend_from_slice(&TRANSFER_SELECTOR);
                    data.extend_from_slice(
                        B256::left_padding_from(recipient.as_slice()).as_slice(),
                    );
                    data.extend_from_slice(&U256::from(1u64).to_be_bytes::<32>());
                    (self.token, U256::ZERO, Bytes::from(data), tx.gas_limit())
                }
            };
            let env = TxEnv::builder()
                .caller(caller)
                .kind(EvmTxKind::Call(to))
                .value(value)
                .data(data)
                .gas_limit(gas)
                .gas_price(0)
                .nonce(tx.nonce)
                .chain_id(Some(CHAIN_ID))
                .build_fill();
            match self.evm.transact(env) {
                Ok(res) => {
                    if !res.result.is_success() {
                        out.failed += 1;
                    }
                    out.gas_used += res.result.tx_gas_used();
                    for (addr, account) in &res.state {
                        if account.is_touched() {
                            touched_accounts.push(*addr);
                            touched_slots
                                .extend(account.changed_storage_slots().map(|(k, _)| (*addr, *k)));
                        }
                    }
                    self.evm.commit(res.state);
                }
                Err(_) => out.failed += 1,
            }
        }
        std::hint::black_box(self.touched_root(touched_accounts, touched_slots));
        out
    }

    fn touched_root(&self, mut accounts: Vec<Address>, slots: Vec<(Address, U256)>) -> B256 {
        let db = &self.evm.ctx.journaled_state.database;
        let mut storage_leaves: Vec<(Nibbles, Vec<u8>)> = slots
            .iter()
            .filter(|(addr, _)| *addr == self.token)
            .map(|(_, slot)| {
                let value = db
                    .cache
                    .accounts
                    .get(&self.token)
                    .and_then(|a| a.storage.get(slot).copied())
                    .unwrap_or_default();
                let mut buf = Vec::new();
                value.encode(&mut buf);
                (Nibbles::unpack(keccak256(slot.to_be_bytes::<32>())), buf)
            })
            .collect();
        storage_leaves.sort_by_key(|leaf| leaf.0);
        storage_leaves.dedup_by(|a, b| a.0 == b.0);
        let token_root = build_root(storage_leaves);

        accounts.sort();
        accounts.dedup();
        let mut leaves: Vec<(Nibbles, Vec<u8>)> = accounts
            .iter()
            .filter_map(|addr| db.cache.accounts.get(addr).map(|a| (addr, a)))
            .map(|(addr, a)| {
                let storage_root = if *addr == self.token { token_root } else { EMPTY_ROOT_HASH };
                let code_hash =
                    if a.info.code_hash == B256::ZERO { KECCAK_EMPTY } else { a.info.code_hash };
                let account = TrieAccount {
                    nonce: a.info.nonce,
                    balance: a.info.balance,
                    storage_root,
                    code_hash,
                };
                let mut buf = Vec::new();
                account.encode(&mut buf);
                (Nibbles::unpack(keccak256(addr)), buf)
            })
            .collect();
        leaves.sort_by_key(|leaf| leaf.0);
        build_root(leaves)
    }
}

fn build_root(leaves: Vec<(Nibbles, Vec<u8>)>) -> B256 {
    let mut hb = HashBuilder::default();
    for (key, value) in &leaves {
        hb.add_leaf(*key, value);
    }
    hb.root()
}

/// Address of account `i`.
pub fn account_address(i: u32) -> Address {
    let mut bytes = [0u8; 20];
    bytes[0] = 0x10;
    bytes[16..].copy_from_slice(&i.to_be_bytes());
    Address::from(bytes)
}

fn address_slot(addr: Address) -> U256 {
    U256::from_be_slice(addr.as_slice())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tx(sender: u32, recipient: u32, nonce: u64, kind: TxKind) -> Tx {
        Tx { sender, recipient, nonce, kind, created_us: 0 }
    }

    #[test]
    fn executes_transfers_and_token_calls() {
        let mut engine = ExecEngine::new(8);
        let txs =
            [tx(0, 1, 0, TxKind::Transfer), tx(0, 2, 1, TxKind::Erc20), tx(3, 0, 0, TxKind::Erc20)];
        let res = engine.execute(txs.iter());
        assert_eq!(res.failed, 0);
        assert_eq!((res.transfers, res.erc20), (1, 2));
        // Two token calls each do two SSTOREs on top of the intrinsic 21000 gas.
        assert!(res.gas_used > 21_000 * 3 + 2 * 5_000);
    }

    #[test]
    fn rejects_out_of_order_nonce() {
        let mut engine = ExecEngine::new(4);
        let res = engine.execute([tx(0, 1, 5, TxKind::Transfer)].iter());
        assert_eq!(res.failed, 1);
    }
}
