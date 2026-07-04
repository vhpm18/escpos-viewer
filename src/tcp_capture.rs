use std::io::Read;
use std::net::{TcpListener, TcpStream};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc::{self, Receiver, Sender},
    Arc,
};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use eframe::egui;
use crate::window_control::WindowControl;

#[derive(Debug, Clone)]
pub struct CapturedJob {
    pub source: String,
    pub bytes: Vec<u8>,
}

pub struct TcpCapture {
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
    rx: Receiver<CapturedJob>,
    local_addr: std::net::SocketAddr,
}

impl TcpCapture {
    pub fn start(
        bind_addr: &str,
        repaint_ctx: Option<egui::Context>,
        window: Option<WindowControl>,
    ) -> std::io::Result<Self> {
        let listener = TcpListener::bind(bind_addr)?;
        let local_addr = listener.local_addr()?;
        listener.set_nonblocking(true)?;

        let (tx, rx) = mpsc::channel::<CapturedJob>();
        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = stop.clone();
        let bind_addr_string = bind_addr.to_string();

        let join = thread::spawn(move || {
            loop {
                if stop_thread.load(Ordering::Relaxed) {
                    break;
                }

                match listener.accept() {
                    Ok((stream, peer)) => {
                        let tx = tx.clone();
                        let source = format!("{} -> {}", peer, bind_addr_string);
                        if let Err(err) =
                            read_one_job(stream, source, tx, repaint_ctx.clone(), window.clone())
                        {
                            let _ = err; // silencioso
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => {
                        // Si el accept falla por otra cosa, salimos para evitar loop caliente.
                        break;
                    }
                }
            }
        });

        Ok(Self {
            stop,
            join: Some(join),
            rx,
            local_addr,
        })
    }

    pub fn try_recv_all(&self) -> Vec<CapturedJob> {
        self.rx.try_iter().collect()
    }

    pub fn local_addr(&self) -> std::net::SocketAddr {
        self.local_addr
    }

    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

impl Drop for TcpCapture {
    fn drop(&mut self) {
        self.stop();
    }
}

fn read_one_job(
    mut stream: TcpStream,
    source: String,
    tx: Sender<CapturedJob>,
    repaint_ctx: Option<egui::Context>,
    window: Option<WindowControl>,
) -> std::io::Result<()> {
    // Normalmente Windows abre conexin, manda bytes y cierra (EOF) por job.
    // Pongo timeout por si el peer se queda abierto.
    // Un timeout muy corto puede partir un ticket en 2 jobs si el POS manda en ráfagas.
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));

    let mut buf = Vec::new();
    let mut tmp = [0u8; 8192];

    loop {
        match stream.read(&mut tmp) {
            Ok(0) => break,
            Ok(n) => buf.extend_from_slice(&tmp[..n]),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
            Err(e) if e.kind() == std::io::ErrorKind::TimedOut => {
                // Consideramos fin de job por inactividad.
                break;
            }
            Err(e) => return Err(e),
        }
    }

    if !buf.is_empty() {
        let _ = tx.send(CapturedJob { source, bytes: buf });
        if let Some(w) = window {
            w.show_and_focus();
        }
        if let Some(ctx) = repaint_ctx {
            ctx.request_repaint();
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::time::Duration;

    #[test]
    fn captures_data_from_tcp_client() {
        let cap = TcpCapture::start("127.0.0.1:0", None, None)
            .expect("failed to start capture");

        let addr = cap.local_addr();
        let mut client = std::net::TcpStream::connect(addr)
            .expect("failed to connect client");
        client.write_all(b"Hello, ESC/POS!")
            .expect("failed to write");
        drop(client);

        std::thread::sleep(Duration::from_millis(200));

        let jobs = cap.try_recv_all();
        assert!(!jobs.is_empty(), "expected at least one captured job");
        assert_eq!(&jobs[0].bytes, b"Hello, ESC/POS!");
    }

    #[test]
    fn capture_returns_empty_when_no_data() {
        let cap = TcpCapture::start("127.0.0.1:0", None, None)
            .expect("failed to start capture");

        let addr = cap.local_addr();
        let _client = std::net::TcpStream::connect(addr)
            .expect("failed to connect");
        // Drop immediately — no data sent, server sees EOF (Ok(0)), no job pushed

        std::thread::sleep(Duration::from_millis(200));
        let jobs = cap.try_recv_all();
        assert!(jobs.is_empty(), "expected no jobs for empty connection");
    }

    #[test]
    fn capture_handles_multiple_clients() {
        let cap = TcpCapture::start("127.0.0.1:0", None, None)
            .expect("failed to start capture");
        let addr = cap.local_addr();

        let mut c1 = std::net::TcpStream::connect(addr)
            .expect("failed to connect");
        c1.write_all(b"Job1 data").expect("write failed");
        drop(c1);

        let mut c2 = std::net::TcpStream::connect(addr)
            .expect("failed to connect");
        c2.write_all(b"Job2 data").expect("write failed");
        drop(c2);

        std::thread::sleep(Duration::from_millis(300));
        let jobs = cap.try_recv_all();
        assert_eq!(jobs.len(), 2, "expected two captured jobs");
        assert_eq!(&jobs[0].bytes, b"Job1 data");
        assert_eq!(&jobs[1].bytes, b"Job2 data");
    }

    #[test]
    fn stop_terminates_cleanly() {
        let cap = TcpCapture::start("127.0.0.1:0", None, None)
            .expect("failed to start capture");
        let addr = cap.local_addr();

        let mut client = std::net::TcpStream::connect(addr)
            .expect("failed to connect");
        client.write_all(b"Test").expect("write failed");
        drop(client);

        std::thread::sleep(Duration::from_millis(200));

        let mut cap = cap;
        cap.stop();

        let jobs = cap.try_recv_all();
        assert!(!jobs.is_empty(), "should have captured the job before stop");
    }
}
