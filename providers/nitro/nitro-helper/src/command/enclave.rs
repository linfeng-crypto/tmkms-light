use crate::enclave_log_server::LogServer;

use crate::config::EnclaveOpt;
use crossbeam_channel::Receiver;
use nitro_cli::common::commands_parser::{EmptyArgs, RunEnclavesArgs};
use nitro_cli::common::json_output::{EnclaveDescribeInfo, EnclaveRunInfo, EnclaveTerminateInfo};
use nitro_cli::common::{enclave_proc_command_send_single, logger, EnclaveProcessCommandType};
use nitro_cli::enclave_proc_comm::{
    enclave_proc_command_send_all, enclave_proc_connect_to_single, enclave_proc_handle_outputs,
    enclave_proc_spawn,
};
use nitro_cli::terminate_all_enclaves;
use std::os::unix::net::UnixStream;

fn run_enclave_daemon(config: &RunEnclavesArgs) -> Result<Option<EnclaveRunInfo>, String> {
    tracing::debug!("run enclave with config: {:?}", config);
    let logger = logger::init_logger().map_err(|_e| "Logger initialization".to_string())?;

    logger
        .update_logger_id(format!("nitro-cli:{}", std::process::id()).as_str())
        .map_err(|_e| "Update CLI Process Logger ID".to_string())?;

    let mut comm = enclave_proc_spawn(&logger)
        .map_err(|_err| "Failed to spawn enclave process".to_string())?;

    enclave_proc_command_send_single(EnclaveProcessCommandType::Run, Some(config), &mut comm)
        .map_err(|_e| ("Failed to send single command".to_string()))?;

    let mut replies: Vec<UnixStream> = vec![comm];
    let output = enclave_proc_handle_outputs::<EnclaveRunInfo>(&mut replies);
    if output.is_empty() {
        return Err("can not get enclave run info".into());
    }
    let (info, status_code) = &output[0];
    if *status_code == 0 {
        Ok(Some(info.clone()))
    } else {
        Err(format!("get enclave info error, {}", status_code))
    }
}

pub fn stop_enclave(enclave_id: Option<String>) -> Result<bool, String> {
    if enclave_id.is_none() {
        terminate_all_enclaves()
            .map_err(|_e| "Failed to terminate all running enclaves".to_string())?;
        return Ok(true);
    }
    let mut comm = enclave_proc_connect_to_single(&enclave_id.unwrap())
        .map_err(|_e| "Failed to connect to enclave process".to_string())?;
    enclave_proc_command_send_single::<EmptyArgs>(
        EnclaveProcessCommandType::Terminate,
        None,
        &mut comm,
    )
    .map_err(|_e| "Failed to send single command".to_string())?;
    let mut replies = vec![comm];
    let output = enclave_proc_handle_outputs::<EnclaveTerminateInfo>(&mut replies);
    if !output.is_empty() {
        let (info, status_code) = &output[0];
        if *status_code != 0 {
            return Err(format!(
                "terminate enclave error with status code: {}",
                status_code
            ));
        } else {
            return Ok(info.terminated);
        }
    }
    Ok(false)
}

pub fn get_enclave_info() -> Result<Vec<EnclaveDescribeInfo>, String> {
    let (comms, _comm_errors) =
        enclave_proc_command_send_all::<EmptyArgs>(EnclaveProcessCommandType::Describe, None)
            .map_err(|_e| {
                "Failed to send DescribeEnclave command to all enclave processes".to_string()
            })?;

    let mut replies: Vec<UnixStream> = comms;
    let info: Vec<_> = enclave_proc_handle_outputs::<EnclaveDescribeInfo>(&mut replies)
        .iter()
        .filter_map(|(info, status_code)| {
            if *status_code == 0 {
                Some(info.clone())
            } else {
                None
            }
        })
        .collect();
    Ok(info)
}

pub fn run_enclave(opt: &EnclaveOpt, stop_receiver: Receiver<()>) -> Result<(), String> {
    // check if the enclave already running
    let enclave_info = get_enclave_info()?;
    if !enclave_info.is_empty() {
        let info = serde_json::to_string_pretty(&enclave_info).expect("get invalid enclave info");
        return Err(format!(
            "the following enclave is already active, please stop and try again:\n{:?}",
            info
        ));
    }
    // lauch enclave server
    tracing::info!("start enclave log server at port {}", opt.log_server_port);
    let enclave_log_server = LogServer::new(
        opt.log_server_port,
        opt.log_to_console,
        opt.log_file.clone(),
    )
    .map_err(|e| format!("{:?}", e))?;

    enclave_log_server.launch();
    // run enclave
    let config = opt.get_run_enclave_args();
    match run_enclave_daemon(&config)? {
        Some(info) => {
            let s = serde_json::to_string_pretty(&info).unwrap();
            tracing::info!("run enclave success:\n{}", s);
            // waiting for stop signal and stop the enclave
            let _ = stop_receiver.recv();
            let _ = stop_enclave(Some(info.enclave_id));
        }
        None => {
            tracing::error!("run enclave failed");
        }
    }
    Ok(())
}
