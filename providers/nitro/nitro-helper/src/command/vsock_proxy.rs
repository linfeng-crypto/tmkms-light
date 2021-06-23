use crate::config::VSockProxyOpt;
use vsock_proxy::starter::Proxy;

pub fn run_vsock_proxy(opt: &VSockProxyOpt) -> Result<(), String> {
    tracing::debug!("run vsock proxy with config: {:?}", opt);
    let remote_addrs = Proxy::parse_addr(&opt.remote_addr, false, false)
        .map_err(|err| format!("Could not parse remote address: {}", err))?;
    let remote_addr = *remote_addrs.get(0).ok_or("No IP address found")?;
    let config_file = Some(opt.config_file.as_str());
    let proxy = Proxy::new(
        opt.local_port,
        remote_addr,
        opt.remote_port,
        opt.num_workers,
        config_file,
        false,
        false,
    )
    .map_err(|err| format!("Could not create proxy: {}", err))?;
    let listener = proxy
        .sock_listen()
        .map_err(|err| format!("Could not listen for connections: {}", err))?;
    tracing::info!("Proxy is now in listening state");
    loop {
        proxy
            .sock_accept(&listener)
            .map_err(|err| format!("Could not accept connection: {}", err))?;
    }
}
