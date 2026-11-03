// E2E bench tool — Windows only (mirrors e2e on Linux/uinput).
//
//   e2e-win listen [--emit]          LL keyboard hook + QPC timestamps.
//                                    client mode: "R <ns> K <vk> <val>" per event.
//                                    --emit (server): non-injected key-downs only,
//                                    "E <ns> <seq>" — pairs with client-side downs.
//   e2e-win skewd [bind]             TCP responder: sends local QPC ns per request.
//   e2e-win skew <host:port> [n]     NTP-style samples: "S <skew_ns> <local_mid_ns>".

#[cfg(windows)]
fn main() {
    e2w::run()
}

#[cfg(not(windows))]
fn main() {
    eprintln!("e2e-win is a Windows test tool");
    std::process::exit(1);
}

#[cfg(windows)]
mod e2w {
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::atomic::{AtomicI64, Ordering};
    use std::time::Duration;

    use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
    use windows_sys::Win32::System::Performance::{
        QueryPerformanceCounter, QueryPerformanceFrequency,
    };
    use windows_sys::Win32::System::Threading::GetCurrentThreadId;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, GetMessageW, SetWindowsHookExW, KBDLLHOOKSTRUCT,
        MSG, WH_KEYBOARD_LL, WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP,
    };

    const LLKHF_INJECTED: u32 = 0x10;

    static FREQ: AtomicI64 = AtomicI64::new(0);
    static EMIT_MODE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    static VK_FILTER: AtomicI64 = AtomicI64::new(-1);
    static SEQ: AtomicI64 = AtomicI64::new(0);

    fn qpc() -> i64 {
        unsafe {
            let mut v = 0i64;
            QueryPerformanceCounter(&mut v);
            v
        }
    }

    fn qpc_ns() -> i64 {
        let f = FREQ.load(Ordering::Relaxed);
        (qpc() as i128 * 1_000_000_000 / f as i128) as i64
    }

    fn init_freq() {
        unsafe {
            let mut f = 0i64;
            QueryPerformanceFrequency(&mut f);
            FREQ.store(f, Ordering::Relaxed);
        }
    }

    unsafe extern "system" fn khook(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
        let ts = qpc_ns();
        if code >= 0 {
            let k = &*(l as *const KBDLLHOOKSTRUCT);
            let injected = k.flags & LLKHF_INJECTED != 0;
            let down = w == WM_KEYDOWN as usize || w == WM_SYSKEYDOWN as usize;
            let up = w == WM_KEYUP as usize || w == WM_SYSKEYUP as usize;
            if down || up {
                if EMIT_MODE.load(Ordering::Relaxed) {
                    let vf = VK_FILTER.load(Ordering::Relaxed);
                    if down && !injected && (vf < 0 || k.vkCode == vf as u32) {
                        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
                        println!("E {} {}", ts, seq);
                    }
                } else {
                    println!("R {} K {} {}", ts, k.vkCode, i32::from(down));
                }
                let _ = std::io::stdout().flush();
            }
        }
        CallNextHookEx(std::ptr::null_mut(), code, w, l)
    }

    fn listen(emit: bool, vk: i64) -> ! {
        init_freq();
        EMIT_MODE.store(emit, Ordering::Relaxed);
        VK_FILTER.store(vk, Ordering::Relaxed);
        unsafe {
            let hook = SetWindowsHookExW(
                WH_KEYBOARD_LL,
                Some(khook),
                std::ptr::null_mut(),
                0,
            );
            if hook.is_null() {
                eprintln!("SetWindowsHookExW failed");
                std::process::exit(1);
            }
            let _tid = GetCurrentThreadId();
            let mut msg: MSG = std::mem::zeroed();
            loop {
                if GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) <= 0 {
                    break;
                }
            }
            std::process::exit(0);
        }
    }

    fn skewd(bind: &str) -> std::io::Result<()> {
        init_freq();
        let l = TcpListener::bind(bind)?;
        eprintln!("skewd listening on {bind}");
        for s in l.incoming() {
            let mut s = s?;
            let ts = qpc_ns();
            let mut buf = [0u8; 8];
            let _ = s.read(&mut buf);
            let _ = s.write_all(&ts.to_be_bytes());
        }
        Ok(())
    }

    fn skew(peer: &str, n: usize) -> std::io::Result<()> {
        init_freq();
        for _ in 0..n {
            let t1 = qpc_ns();
            let mut s = TcpStream::connect(peer)?;
            s.write_all(&1u64.to_be_bytes())?;
            let mut b = [0u8; 8];
            s.read_exact(&mut b)?;
            let t3 = qpc_ns();
            let remote = i64::from_be_bytes(b);
            let mid = (t1 + t3) / 2;
            let rtt = t3 - t1;
            println!("S {} {} # rtt={}", remote - mid, mid, rtt);
            std::thread::sleep(Duration::from_millis(50));
        }
        Ok(())
    }

    pub fn run() {
        let a: Vec<String> = std::env::args().skip(1).collect();
        let usage = "usage: e2e-win listen [--emit] [--vk <dec>] | skewd [bind:port] | skew <host:port> [n]";
        match a.first().map(String::as_str) {
            Some("listen") => {
                let vk = a
                    .iter()
                    .position(|x| x == "--vk")
                    .and_then(|i| a.get(i + 1))
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(-1);
                listen(a.iter().any(|x| x == "--emit"), vk)
            }
            Some("skewd") => {
                let bind = a.get(1).map(String::as_str).unwrap_or("0.0.0.0:24900");
                let _ = skewd(bind);
            }
            Some("skew") => {
                let peer = match a.get(1) {
                    Some(p) => p.clone(),
                    None => {
                        eprintln!("{usage}");
                        std::process::exit(2);
                    }
                };
                let n = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(8);
                let _ = skew(&peer, n);
            }
            _ => eprintln!("{usage}"),
        }
    }
}
