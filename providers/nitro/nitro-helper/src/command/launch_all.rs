use crate::command::nitro_enclave::run_vsock_proxy;
use crate::command::nitro_enclave::{describe_enclave, run_enclave};
use crate::command::start;
use crate::config::Config;
use crossbeam_channel::{bounded, Receiver, Sender};
use std::thread::{self, sleep};
use std::time::Duration;

pub struct Launcher {
    config: Config,
    stop_enclave_sender: Sender<()>,
    stop_vsock_proxy_sender: Sender<()>,
}

impl Launcher {
    /// create a new launcher, stop_enclave_sender: before the launcher exit, send the signal to
    /// the subprocess so that it can stop gracefully.
    pub fn new(
        config: Config,
        stop_enclave_sender: Sender<()>,
        stop_vsock_proxy_sender: Sender<()>,
    ) -> Self {
        Self {
            config,
            stop_enclave_sender,
            stop_vsock_proxy_sender,
        }
    }

    /// 1. run enclave
    /// 2. launch proxy
    /// 3. start helper
    pub fn run(
        &self,
        stop_enclave_receiver: Receiver<()>,
        stop_vsock_proxy_receiver: Receiver<()>,
    ) -> Result<(), String> {
        // start enclave
        let enclave_config = self.config.enclave.clone();
        let t1 = thread::spawn(move || {
            tracing::info!("starting enclave ...");
            if let Err(e) = run_enclave(&enclave_config, stop_enclave_receiver) {
                tracing::error!("enclave error: {:?}", e);
                std::process::exit(1)
            }
        });

        // launch proxy
        let proxy_config = self.config.vsock_proxy.clone();
        let stop_enclave_sender = self.stop_enclave_sender.clone();
        let t2 = thread::spawn(move || {
            tracing::info!("starting vsock proxy");
            if let Err(e) = run_vsock_proxy(&proxy_config, stop_vsock_proxy_receiver) {
                tracing::error!("vsock proxy error: {:?}", e);
                let _ = stop_enclave_sender.send(());
                std::process::exit(1)
            }
        });

        // run helper
        // check if enclave is running
        tracing::info!("starting helper...");
        let timeout = 15;
        let mut t = 0;
        let cid = loop {
            let enclave_info = describe_enclave()?;
            if enclave_info.is_empty() {
                tracing::error!("can't find running enclave");
            } else {
                break enclave_info[0].enclave_cid;
            }
            t += 1;
            if t >= timeout {
                return Err("can't find running enclave".to_string());
            }
            sleep(Duration::from_secs(1));
        };

        let stop_enclave_sender = self.stop_enclave_sender.clone();
        let stop_vsock_proxy_sender = self.stop_vsock_proxy_sender.clone();
        let sign_config = self.config.sign_opt.clone();
        let _t3 = thread::spawn(move || {
            if let Err(e) = start(&sign_config, Some(cid as u32)) {
                tracing::error!("{}", e);
                tracing::debug!("send close enclave signal");
                let _ = stop_enclave_sender.send(());
                let _ = stop_vsock_proxy_sender.send(());
                std::process::exit(1)
            }
        });
        let stop_enclave_sender = self.stop_enclave_sender.clone();
        let stop_vsock_proxy_sender = self.stop_vsock_proxy_sender.clone();
        ctrlc::set_handler(move || {
            tracing::debug!("get Ctrl-C signal, send close enclave signal");
            let _ = stop_enclave_sender.send(());
            let _ = stop_vsock_proxy_sender.send(());
        })
        .map_err(|_| "Error to set Ctrl-C channel".to_string())?;
        let _ = t1.join();
        let _ = t2.join();
        // let _ = _t3.join();
        Ok(())
    }
}

impl Drop for Launcher {
    fn drop(&mut self) {
        let _ = self.stop_enclave_sender.send(());
        let _ = self.stop_vsock_proxy_sender.send(());
    }
}

pub fn launch_all(config: Config) -> Result<(), String> {
    // run enclave
    let (sender1, receiver1) = bounded(1);
    let (sender2, receiver2) = bounded(1);
    let launcher = Launcher::new(config, sender1, sender2);
    launcher.run(receiver1, receiver2)?;
    Ok(())
}
