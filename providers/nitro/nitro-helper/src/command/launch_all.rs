use crate::command::nitro_enclave::run_vsock_proxy;
use crate::command::nitro_enclave::{describe_enclave, run_enclave};
use crate::command::start;
use crate::config::{EnclaveConfig, NitroSignOpt};
use std::sync::mpsc::{channel, Sender};
use std::thread::{self, sleep};
use std::time::Duration;

pub struct Launcher {
    tmkms_config: NitroSignOpt,
    enclave_config: EnclaveConfig,
    stop_senders: Vec<Sender<()>>,
}

impl Launcher {
    /// create a new launcher, stop_enclave_sender: before the launcher exit, send the signal to
    /// the subprocess so that it can stop gracefully.
    pub fn new(tmkms_config: NitroSignOpt, enclave_config: EnclaveConfig) -> Self {
        Self {
            tmkms_config,
            enclave_config,
            stop_senders: vec![],
        }
    }

    /// 1. run enclave
    /// 2. launch proxy
    /// 3. start helper
    pub fn run(&mut self) -> Result<(), String> {
        // create stop signal (tx,rx)
        let (tx1, rx1) = channel();
        let (tx2, rx2) = channel();
        let (tx3, rx3) = channel();
        self.stop_senders.push(tx1);
        self.stop_senders.push(tx2);
        self.stop_senders.push(tx3);

        let mut threads = vec![];

        // start enclave
        let enclave_config = self.enclave_config.enclave.clone();
        let stop_senders = self.stop_senders.clone();
        let t1 = thread::spawn(move || {
            tracing::info!("starting enclave ...");
            if let Err(e) = run_enclave(&enclave_config, rx1) {
                tracing::error!("enclave error: {:?}", e);
                for tx in stop_senders {
                    let _ = tx.send(());
                }
            }
        });
        threads.push(t1);

        // launch proxy
        let proxy_config = self.enclave_config.vsock_proxy.clone();
        let stop_senders = self.stop_senders.clone();
        let t2 = thread::spawn(move || {
            tracing::info!("starting vsock proxy");
            if let Err(e) = run_vsock_proxy(&proxy_config, rx2) {
                tracing::error!("vsock proxy error: {:?}", e);
                for tx in stop_senders {
                    let _ = tx.send(());
                }
            }
        });
        threads.push(t2);

        // run helper
        // check if enclave is running
        tracing::info!("starting helper, waiting for the enclave running...");
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

        let tmkms_config = self.tmkms_config.clone();
        let stop_senders = self.stop_senders.clone();
        let t3 = thread::spawn(move || {
            if let Err(e) = start(&tmkms_config, Some(cid as u32), rx3) {
                tracing::error!("{}", e);
                for tx in stop_senders {
                    let _ = tx.send(());
                }
            }
        });
        threads.push(t3);

        // when get the ctrlc signal, send stop signal
        let stop_senders = self.stop_senders.clone();
        ctrlc::set_handler(move || {
            tracing::debug!("get Ctrl-C signal, send close enclave signal");
            for tx in stop_senders.iter() {
                let _ = tx.send(());
            }
        })
        .map_err(|_| "Error to set Ctrl-C channel".to_string())?;

        for t in threads.into_iter() {
            let _ = t.join();
        }
        Ok(())
    }
}

pub fn launch_all(tmkms_config: NitroSignOpt, enclave_config: EnclaveConfig) -> Result<(), String> {
    let mut launcher = Launcher::new(tmkms_config, enclave_config);
    launcher.run()?;
    Ok(())
}
