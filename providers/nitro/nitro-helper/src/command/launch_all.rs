use crate::command::enclave::{get_enclave_info, run_enclave};
use crate::command::helper::start;
use crate::command::vsock_proxy::run_vsock_proxy;
use crate::config::Config;
use crossbeam_channel::{bounded, Receiver, Sender};
use std::thread::{self, sleep};
use std::time::Duration;
use tracing::Level;
use tracing_subscriber::FmtSubscriber;

pub struct Launcher {
    log_level: Level,
    config: Config,
    stop_enclave_sender: Sender<()>,
}

impl Launcher {
    pub fn new(log_level: Level, config: Config, stop_enclave_sender: Sender<()>) -> Self {
        Self {
            log_level,
            config,
            stop_enclave_sender,
        }
    }

    /// 1. run enclave
    /// 2. launch proxy
    /// 3. start helper
    pub fn run(&self, receiver: Receiver<()>) -> Result<(), String> {
        let subscriber = FmtSubscriber::builder()
            .with_max_level(self.log_level)
            .finish();
        tracing::subscriber::set_global_default(subscriber)
            .map_err(|e| format!("setting default subscriber failed: {:?}", e))?;

        // start enclave
        let enclave_config = self.config.enclave.clone();
        let t1 = thread::spawn(move || {
            if let Err(e) = run_enclave(&enclave_config, receiver) {
                tracing::error!("enclave error: {:?}", e);
                std::process::exit(1)
            }
        });

        // launch proxy
        let proxy_config = self.config.vsock_proxy.clone();
        let sender = self.stop_enclave_sender.clone();
        let _t2 = thread::spawn(move || {
            if let Err(e) = run_vsock_proxy(&proxy_config) {
                tracing::error!("vsock proxy error: {:?}", e);
                let _ = sender.send(());
                std::process::exit(1)
            }
        });

        // run helper
        // get cid
        tracing::info!("start helper...");
        sleep(Duration::from_secs(5));
        let enclave_info = get_enclave_info()?;
        if enclave_info.is_empty() {
            tracing::error!("can't find running enclave");
            let _ = self.stop_enclave_sender.send(());
            return Ok(());
        }
        let sender = self.stop_enclave_sender.clone();
        let sign_config = self.config.sign_opt.clone();
        let _t3 = thread::spawn(move || {
            let cid = enclave_info[0].enclave_cid;
            if let Err(e) = start(&sign_config, Some(cid as u32)) {
                tracing::error!("{}", e);
                tracing::debug!("send close enclave signal");
                let _ = sender.send(());
                std::process::exit(1)
            }
        });
        let sender = self.stop_enclave_sender.clone();
        ctrlc::set_handler(move || {
            tracing::debug!("get Ctrl-C signal, send close enclave signal");
            let _ = sender.send(());
        })
        .map_err(|_| "Error to set Ctrl-C channel".to_string())?;
        let _ = t1.join();
        // let _ = _t2.join();
        // let _ = _t3.join();
        Ok(())
    }
}

impl Drop for Launcher {
    fn drop(&mut self) {
        let _ = self.stop_enclave_sender.send(());
    }
}

pub fn launch_all(log_level: Level, config: Config) -> Result<(), String> {
    // run enclave
    let (sender, receiver) = bounded(1);
    let launcher = Launcher::new(log_level, config, sender);
    launcher.run(receiver)?;
    Ok(())
}
