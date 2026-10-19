//! Signature costs measured on this machine with the same libraries reth-bsc uses.
//!
//! The simulator charges these costs to the virtual clock of every node that performs the
//! operation. Each operation is measured once at startup instead of being re-run for every
//! simulated message, which would make a 45-validator run take far longer than the simulated
//! time.

use std::time::Instant;

use blst::{
    min_pk::{AggregateSignature, PublicKey, SecretKey, Signature},
    BLST_ERROR,
};
use ed25519_dalek::{Signer, SigningKey, Verifier};
use secp256k1::{Message, Secp256k1};
use serde::Serialize;

/// Domain separation tag of the BLS signatures, the same one Parlia votes use.
const BLS_DST: &[u8] = b"BLS_SIG_BLS12381G2_XMD:SHA-256_SSWU_RO_POP_";

/// Number of signatures verified together with `ed25519_dalek::verify_batch`.
pub const ED25519_BATCH: usize = 64;

/// Per-operation CPU cost in microseconds.
#[derive(Debug, Clone, Serialize)]
pub struct CryptoCosts {
    /// Ed25519 signing.
    pub ed_sign: f64,
    /// Ed25519 verification of a single signature.
    pub ed_verify: f64,
    /// Ed25519 verification amortized over a group of [`ED25519_BATCH`] signatures.
    pub ed_verify_batched: f64,
    /// BLS signing.
    pub bls_sign: f64,
    /// BLS verification of a single signature.
    pub bls_verify: f64,
    /// BLS `fast_aggregate_verify` over a quorum of public keys, including key aggregation.
    pub bls_agg_verify: f64,
    /// Adding one signature to an aggregate.
    pub bls_agg_add: f64,
    /// secp256k1 recoverable signing (Parlia block seal).
    pub ecdsa_sign: f64,
    /// secp256k1 public key recovery (sender recovery, Parlia seal check).
    pub ecdsa_recover: f64,
}

impl CryptoCosts {
    /// Measures every operation on this machine. `quorum` is the number of keys aggregated in
    /// certificates.
    pub fn calibrate(quorum: usize) -> Self {
        let msg = [7u8; 32];

        let ed_keys: Vec<SigningKey> =
            (0..ED25519_BATCH).map(|i| SigningKey::from_bytes(&seed(i))).collect();
        let ed_sigs: Vec<_> = ed_keys.iter().map(|k| k.sign(&msg)).collect();
        let ed_vks: Vec<_> = ed_keys.iter().map(|k| k.verifying_key()).collect();
        let ed_sign = time_per_op(200, |i| {
            std::hint::black_box(ed_keys[i % ED25519_BATCH].sign(&msg));
        });
        let ed_verify = time_per_op(200, |i| {
            let j = i % ED25519_BATCH;
            assert!(ed_vks[j].verify(&msg, &ed_sigs[j]).is_ok());
        });
        let msgs: Vec<&[u8]> = vec![&msg[..]; ED25519_BATCH];
        let ed_verify_batched = time_per_op(20, |_| {
            assert!(ed25519_dalek::verify_batch(&msgs, &ed_sigs, &ed_vks).is_ok());
        }) / ED25519_BATCH as f64;

        let bls_keys: Vec<SecretKey> =
            (0..quorum).map(|i| SecretKey::key_gen(&seed(i), &[]).expect("valid ikm")).collect();
        let bls_pks: Vec<PublicKey> = bls_keys.iter().map(|k| k.sk_to_pk()).collect();
        let bls_sigs: Vec<Signature> =
            bls_keys.iter().map(|k| k.sign(&msg, BLS_DST, &[])).collect();
        let bls_sign = time_per_op(50, |i| {
            std::hint::black_box(bls_keys[i % quorum].sign(&msg, BLS_DST, &[]));
        });
        let bls_verify = time_per_op(50, |i| {
            let j = i % quorum;
            let res = bls_sigs[j].verify(true, &msg, BLS_DST, &[], &bls_pks[j], true);
            assert_eq!(res, BLST_ERROR::BLST_SUCCESS);
        });
        let sig_refs: Vec<&Signature> = bls_sigs.iter().collect();
        let pk_refs: Vec<&PublicKey> = bls_pks.iter().collect();
        let agg =
            AggregateSignature::aggregate(&sig_refs, true).expect("valid sigs").to_signature();
        let bls_agg_verify = time_per_op(30, |_| {
            let res = agg.fast_aggregate_verify(true, &msg, BLS_DST, &pk_refs);
            assert_eq!(res, BLST_ERROR::BLST_SUCCESS);
        });
        let bls_agg_add = time_per_op(10, |_| {
            std::hint::black_box(AggregateSignature::aggregate(&sig_refs, false).expect("valid"));
        }) / quorum as f64;

        let secp = Secp256k1::new();
        let sk = secp256k1::SecretKey::from_slice(&seed(1)).expect("valid key");
        let message = Message::from_digest(msg);
        let rec_sig = secp.sign_ecdsa_recoverable(&message, &sk);
        let ecdsa_sign = time_per_op(200, |_| {
            std::hint::black_box(secp.sign_ecdsa_recoverable(&message, &sk));
        });
        let ecdsa_recover = time_per_op(200, |_| {
            std::hint::black_box(secp.recover_ecdsa(&message, &rec_sig).expect("recoverable"));
        });

        Self {
            ed_sign,
            ed_verify,
            ed_verify_batched,
            bls_sign,
            bls_verify,
            bls_agg_verify,
            bls_agg_add,
            ecdsa_sign,
            ecdsa_recover,
        }
    }
}

fn seed(i: usize) -> [u8; 32] {
    let mut out = [0x42u8; 32];
    out[..8].copy_from_slice(&(i as u64 + 1).to_le_bytes());
    out
}

/// Runs `op` `iters` times after a warm-up and returns the mean duration in microseconds.
fn time_per_op(iters: usize, mut op: impl FnMut(usize)) -> f64 {
    for i in 0..iters.div_ceil(10) {
        op(i);
    }
    let start = Instant::now();
    for i in 0..iters {
        op(i);
    }
    start.elapsed().as_secs_f64() * 1e6 / iters as f64
}
