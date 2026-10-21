//! Real-network transport: every validator gets an agent process in its own network namespace,
//! and protocol messages travel as real bytes over TCP between the agents.
//!
//! The protocol logic stays in the simulator process (the controller), which runs in wall-clock
//! time. When the protocol sends a message, the controller tells the sender's agent to write a
//! frame of the message's size to each recipient's agent. The recipient's agent reports when the
//! last byte arrived, and the controller delivers the message one link latency later. Upload
//! bandwidth is enforced by an HTB qdisc on each namespace's interface ([`Namespaces`]), so
//! serialization, TCP, the kernel and contention between concurrent flows are all real.
//!
//! Latency and jitter are added in user space because the kernel of the test container has no
//! `netem`. TCP therefore sees a sub-millisecond RTT, so congestion-window effects of real
//! long-haul links are not reproduced.

use std::{
    io::{BufReader, BufWriter, Read, Write},
    net::{Ipv4Addr, SocketAddrV4, TcpListener, TcpStream},
    os::unix::net::{UnixListener, UnixStream},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc, Arc,
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

/// TCP port every agent listens on.
const AGENT_PORT: u16 = 7700;
/// Frame header on the wire: message id, sender, payload length.
const FRAME_HEADER: usize = 8 + 2 + 4;
/// Shortest pause the paced writer sleeps for.
const MIN_PACING_SLEEP: Duration = Duration::from_micros(200);
/// How long the controller waits for the agents to form a full mesh.
const STARTUP_TIMEOUT: Duration = Duration::from_secs(60);

/// Where and how to launch the agents.
#[derive(Debug, Clone)]
pub struct RealNetSpec {
    /// Directory for the control socket.
    pub dir: PathBuf,
    /// This binary, run with the `agent` subcommand.
    pub exe: PathBuf,
    /// Upload rate in Mbit/s at which agents pace one frame after another, or `None` to write
    /// to every peer at once.
    pub serial_mbps: Option<f64>,
}

/// A message's last byte reached its recipient's agent.
#[derive(Debug, Clone, Copy)]
pub struct Arrival {
    /// Controller-assigned message id.
    pub msg: u64,
    /// Recipient validator.
    pub dst: usize,
    /// Wall-clock arrival, microseconds since the Unix epoch.
    pub recv_us: u64,
}

/// Address of validator `id`'s agent.
pub fn agent_addr(id: usize) -> SocketAddrV4 {
    let host = id + 1;
    SocketAddrV4::new(Ipv4Addr::new(10, 77, (host / 256) as u8, (host % 256) as u8), AGENT_PORT)
}

/// Wall clock in microseconds since the Unix epoch. All agents run on the same host, so their
/// clocks agree.
pub fn wall_us() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).expect("clock after epoch").as_micros() as u64
}

/// The running agents of one protocol run, as seen from the controller.
#[derive(Debug)]
pub struct Cluster {
    children: Vec<Child>,
    commands: Vec<BufWriter<UnixStream>>,
    /// Arrivals reported by all agents.
    pub arrivals: mpsc::Receiver<Arrival>,
    start_us: u64,
}

impl Cluster {
    /// Starts one agent per validator in namespaces `dxnet0..` (see [`Namespaces`]) and
    /// waits until every agent is connected to every other.
    pub fn launch(spec: &RealNetSpec, n: usize) -> Self {
        std::fs::create_dir_all(&spec.dir).expect("control directory");
        let sock = spec.dir.join(format!("ctl-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&sock);
        let listener = UnixListener::bind(&sock).expect("control socket");
        let children = (0..n)
            .map(|id| {
                Command::new("ip")
                    .args(["netns", "exec", &format!("dxnet{id}")])
                    .arg(&spec.exe)
                    .args(["agent", "--id", &id.to_string(), "--n", &n.to_string(), "--ctl"])
                    .arg(&sock)
                    .args(spec.serial_mbps.map(|m| format!("--serial-mbps={m}")))
                    .stdin(Stdio::null())
                    .spawn()
                    .expect("agent process")
            })
            .collect();
        let mut streams: Vec<Option<UnixStream>> = (0..n).map(|_| None).collect();
        listener.set_nonblocking(true).expect("nonblocking listener");
        let deadline = Instant::now() + STARTUP_TIMEOUT;
        let mut ready = 0;
        while ready < n {
            assert!(Instant::now() < deadline, "agents did not form a full mesh in time");
            match listener.accept() {
                Ok((mut s, _)) => {
                    s.set_nonblocking(false).expect("blocking stream");
                    let mut hello = [0u8; 2];
                    s.read_exact(&mut hello).expect("agent hello");
                    let id = u16::from_le_bytes(hello) as usize;
                    let mut ok = [0u8; 1];
                    s.read_exact(&mut ok).expect("agent ready");
                    streams[id] = Some(s);
                    ready += 1;
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(5));
                }
                Err(e) => panic!("control accept failed: {e}"),
            }
        }
        let _ = std::fs::remove_file(&sock);
        let (tx, arrivals) = mpsc::channel();
        let commands = streams
            .into_iter()
            .map(|s| {
                let s = s.expect("every agent connected");
                let reader = s.try_clone().expect("clone control stream");
                let tx = tx.clone();
                thread::spawn(move || read_arrivals(reader, &tx));
                BufWriter::new(s)
            })
            .collect();
        Self { children, commands, arrivals, start_us: wall_us() }
    }

    /// Microseconds since the run started.
    pub fn now(&self) -> u64 {
        wall_us().saturating_sub(self.start_us)
    }

    /// Converts an arrival's wall-clock time to run time.
    pub const fn to_run_time(&self, wall: u64) -> u64 {
        wall.saturating_sub(self.start_us)
    }

    /// Has `from`'s agent send `bytes` (message `msg`) to each of `dsts`.
    pub fn dispatch(&mut self, from: usize, msg: u64, dsts: &[usize], bytes: u64) {
        let w = &mut self.commands[from];
        let mut buf = Vec::with_capacity(14 + 2 * dsts.len());
        buf.extend_from_slice(&msg.to_le_bytes());
        buf.extend_from_slice(&(bytes as u32).to_le_bytes());
        buf.extend_from_slice(&(dsts.len() as u16).to_le_bytes());
        for &d in dsts {
            buf.extend_from_slice(&(d as u16).to_le_bytes());
        }
        w.write_all(&buf).expect("command to agent");
        w.flush().expect("command to agent");
    }
}

impl Drop for Cluster {
    fn drop(&mut self) {
        for c in &mut self.children {
            let _ = c.kill();
        }
        for c in &mut self.children {
            let _ = c.wait();
        }
        // `ip netns exec` does not forward the signal, so make sure no agent outlives the run.
        let _ = Command::new("pkill").args(["-f", "dx-demo agent"]).status();
    }
}

fn read_arrivals(stream: UnixStream, tx: &mpsc::Sender<Arrival>) {
    let mut r = BufReader::new(stream);
    let mut rec = [0u8; 18];
    while r.read_exact(&mut rec).is_ok() {
        let arrival = Arrival {
            msg: u64::from_le_bytes(rec[0..8].try_into().expect("8 bytes")),
            dst: u16::from_le_bytes(rec[8..10].try_into().expect("2 bytes")) as usize,
            recv_us: u64::from_le_bytes(rec[10..18].try_into().expect("8 bytes")),
        };
        if tx.send(arrival).is_err() {
            return;
        }
    }
}

/// Agent process for validator `id`: keeps a TCP connection to every other agent, writes frames
/// on the controller's command and reports every frame it receives. With `serial_mbps` set, one
/// thread writes all frames in command order and waits after each until the uplink would have
/// sent it at that rate.
pub fn agent_main(id: usize, n: usize, ctl: &Path, serial_mbps: Option<f64>) {
    let listener = TcpListener::bind(agent_addr(id)).expect("agent listen");
    let incoming = Arc::new(AtomicUsize::new(0));
    let (note_tx, note_rx) = mpsc::channel::<(u64, u64)>();
    {
        let incoming = Arc::clone(&incoming);
        let note_tx = note_tx.clone();
        thread::spawn(move || {
            for conn in listener.incoming() {
                let Ok(conn) = conn else { continue };
                conn.set_nodelay(true).expect("nodelay");
                incoming.fetch_add(1, Ordering::SeqCst);
                let note_tx = note_tx.clone();
                thread::spawn(move || read_frames(conn, &note_tx));
            }
        });
    }
    // Outgoing connection to every peer. By default each has its own writer thread so a large
    // frame to one peer does not hold up the others, and the HTB qdisc shares the uplink between
    // them; in serial mode a single paced thread writes to all of them.
    let conns: Vec<Option<TcpStream>> = (0..n)
        .map(|peer| {
            (peer != id).then(|| {
                let conn = loop {
                    match TcpStream::connect(agent_addr(peer)) {
                        Ok(c) => break c,
                        Err(_) => thread::sleep(Duration::from_millis(20)),
                    }
                };
                conn.set_nodelay(true).expect("nodelay");
                conn
            })
        })
        .collect();
    let mut writers: Vec<Option<mpsc::Sender<(u64, u32)>>> = (0..n).map(|_| None).collect();
    let serial = serial_mbps.map(|mbps| {
        let (tx, rx) = mpsc::channel();
        let conns: Vec<_> =
            conns.iter().map(|c| c.as_ref().map(|c| c.try_clone().expect("clone"))).collect();
        thread::spawn(move || write_frames_paced(conns, id, mbps, &rx));
        tx
    });
    if serial.is_none() {
        for (slot, conn) in writers.iter_mut().zip(conns) {
            if let Some(conn) = conn {
                let (tx, rx) = mpsc::channel();
                thread::spawn(move || write_frames(conn, id, &rx));
                *slot = Some(tx);
            }
        }
    }
    while incoming.load(Ordering::SeqCst) < n - 1 {
        thread::sleep(Duration::from_millis(5));
    }
    let mut ctl_stream = UnixStream::connect(ctl).expect("control socket");
    ctl_stream.write_all(&(id as u16).to_le_bytes()).expect("hello");
    ctl_stream.write_all(&[1]).expect("ready");
    let notifier = ctl_stream.try_clone().expect("clone control stream");
    thread::spawn(move || write_notes(notifier, id, &note_rx));

    let mut r = BufReader::new(ctl_stream);
    let mut head = [0u8; 14];
    while r.read_exact(&mut head).is_ok() {
        let msg = u64::from_le_bytes(head[0..8].try_into().expect("8 bytes"));
        let bytes = u32::from_le_bytes(head[8..12].try_into().expect("4 bytes"));
        let count = u16::from_le_bytes(head[12..14].try_into().expect("2 bytes")) as usize;
        let mut dsts = vec![0u8; 2 * count];
        if r.read_exact(&mut dsts).is_err() {
            break;
        }
        for d in dsts.chunks_exact(2) {
            let dst = u16::from_le_bytes([d[0], d[1]]) as usize;
            if let Some(serial) = &serial {
                let _ = serial.send((msg, bytes, dst));
            } else if let Some(Some(w)) = writers.get(dst) {
                let _ = w.send((msg, bytes));
            }
        }
    }
    std::process::exit(0);
}

fn write_frames(conn: TcpStream, from: usize, rx: &mpsc::Receiver<(u64, u32)>) {
    let mut w = BufWriter::with_capacity(64 * 1024, conn);
    while let Ok((msg, bytes)) = rx.recv() {
        if write_frame(&mut w, from, msg, bytes).is_err() {
            return;
        }
    }
}

/// Writes every frame in the order it was queued, to whichever peer it is for, and after each
/// one waits until an uplink of `mbps` would have finished sending it.
fn write_frames_paced(
    conns: Vec<Option<TcpStream>>,
    from: usize,
    mbps: f64,
    rx: &mpsc::Receiver<(u64, u32, usize)>,
) {
    let mut writers: Vec<Option<BufWriter<TcpStream>>> =
        conns.into_iter().map(|c| c.map(|c| BufWriter::with_capacity(64 * 1024, c))).collect();
    let bytes_per_us = mbps / 8.0;
    let mut link_free = Instant::now();
    while let Ok((msg, bytes, dst)) = rx.recv() {
        let Some(Some(w)) = writers.get_mut(dst) else { continue };
        let start = link_free.max(Instant::now());
        if write_frame(w, from, msg, bytes).is_err() {
            return;
        }
        link_free = start + Duration::from_micros((bytes as f64 / bytes_per_us) as u64);
        // Short waits are skipped (sleep overshoots them); `link_free` still accounts for them,
        // so the next long wait covers the debt.
        if let Some(wait) = link_free.checked_duration_since(Instant::now()) {
            if wait >= MIN_PACING_SLEEP {
                thread::sleep(wait);
            }
        }
    }
}

fn write_frame(
    w: &mut BufWriter<TcpStream>,
    from: usize,
    msg: u64,
    bytes: u32,
) -> std::io::Result<()> {
    const ZEROS: [u8; 16 * 1024] = [0; 16 * 1024];
    let payload = (bytes as usize).saturating_sub(FRAME_HEADER);
    let mut head = [0u8; FRAME_HEADER];
    head[0..8].copy_from_slice(&msg.to_le_bytes());
    head[8..10].copy_from_slice(&(from as u16).to_le_bytes());
    head[10..14].copy_from_slice(&(payload as u32).to_le_bytes());
    w.write_all(&head)?;
    let mut left = payload;
    while left > 0 {
        let chunk = left.min(ZEROS.len());
        w.write_all(&ZEROS[..chunk])?;
        left -= chunk;
    }
    w.flush()
}

fn read_frames(conn: TcpStream, note_tx: &mpsc::Sender<(u64, u64)>) {
    let mut r = BufReader::with_capacity(64 * 1024, conn);
    let mut head = [0u8; FRAME_HEADER];
    let mut sink = vec![0u8; 64 * 1024];
    while r.read_exact(&mut head).is_ok() {
        let msg = u64::from_le_bytes(head[0..8].try_into().expect("8 bytes"));
        let mut left = u32::from_le_bytes(head[10..14].try_into().expect("4 bytes")) as usize;
        while left > 0 {
            let chunk = left.min(sink.len());
            if r.read_exact(&mut sink[..chunk]).is_err() {
                return;
            }
            left -= chunk;
        }
        if note_tx.send((msg, wall_us())).is_err() {
            return;
        }
    }
}

fn write_notes(stream: UnixStream, id: usize, rx: &mpsc::Receiver<(u64, u64)>) {
    let mut w = BufWriter::new(stream);
    while let Ok(first) = rx.recv() {
        let mut next = Some(first);
        while let Some((msg, recv_us)) = next {
            let mut rec = [0u8; 18];
            rec[0..8].copy_from_slice(&msg.to_le_bytes());
            rec[8..10].copy_from_slice(&(id as u16).to_le_bytes());
            rec[10..18].copy_from_slice(&recv_us.to_le_bytes());
            if w.write_all(&rec).is_err() {
                return;
            }
            next = rx.try_recv().ok();
        }
        if w.flush().is_err() {
            return;
        }
    }
}

/// Network namespaces `dxnet0..n` on a shared bridge, each with an HTB qdisc capping its upload
/// at `bandwidth_mbps`. Removed again on drop.
#[derive(Debug)]
pub struct Namespaces {
    n: usize,
}

impl Namespaces {
    /// Creates the namespaces, replacing leftovers from an earlier run.
    pub fn create(n: usize, bandwidth_mbps: f64) -> Self {
        Self::remove(n.max(64));
        run_ip(&["link", "add", "dxbr", "type", "bridge"]);
        run_ip(&["link", "set", "dxbr", "up"]);
        let rate = format!("{}mbit", bandwidth_mbps.round() as u64);
        for id in 0..n {
            let ns = format!("dxnet{id}");
            let veth = format!("dxv{id}");
            let addr = agent_addr(id);
            run_ip(&["netns", "add", &ns]);
            run_ip(&["link", "add", &veth, "type", "veth", "peer", "name", "eth0", "netns", &ns]);
            run_ip(&["link", "set", &veth, "master", "dxbr", "up"]);
            run_ip(&["-n", &ns, "addr", "add", &format!("{}/16", addr.ip()), "dev", "eth0"]);
            run_ip(&["-n", &ns, "link", "set", "eth0", "up"]);
            run_ip(&["-n", &ns, "link", "set", "lo", "up"]);
            run_tc(&[
                "-n", &ns, "qdisc", "add", "dev", "eth0", "root", "handle", "1:", "htb", "default",
                "10",
            ]);
            run_tc(&[
                "-n", &ns, "class", "add", "dev", "eth0", "parent", "1:", "classid", "1:10", "htb",
                "rate", &rate, "ceil", &rate, "burst", "64k", "cburst", "64k",
            ]);
        }
        Self { n }
    }

    /// Deletes the veth pairs explicitly as well: a namespace is torn down asynchronously, and
    /// until it is gone its veth peer keeps the name the next run wants.
    fn remove(n: usize) {
        for id in 0..n {
            let _ = Command::new("ip")
                .args(["link", "del", &format!("dxv{id}")])
                .stderr(Stdio::null())
                .status();
            let _ = Command::new("ip")
                .args(["netns", "del", &format!("dxnet{id}")])
                .stderr(Stdio::null())
                .status();
        }
        let _ = Command::new("ip").args(["link", "del", "dxbr"]).stderr(Stdio::null()).status();
    }
}

impl Drop for Namespaces {
    fn drop(&mut self) {
        Self::remove(self.n);
    }
}

fn run_ip(args: &[&str]) {
    let ok = Command::new("ip").args(args).status().is_ok_and(|s| s.success());
    assert!(ok, "ip {} failed", args.join(" "));
}

fn run_tc(args: &[&str]) {
    let ok =
        Command::new("tc").args(args).stderr(Stdio::null()).status().is_ok_and(|s| s.success());
    assert!(ok, "tc {} failed", args.join(" "));
}
