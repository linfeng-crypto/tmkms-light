use crate::shared::AwsCredentials;
use aws_sdk_kms::config::Config;
use aws_sdk_kms::Credentials as KmsCredentials;
use aws_sdk_kms::{Client as KmsClient, Region};
use ed25519_dalek::{Keypair, PublicKey};
use rand_core::OsRng;
use std::{fs::OpenOptions, io::Write, os::unix::fs::OpenOptionsExt, path::Path};
use tokio::runtime::Runtime;

// TODO: use aws-rust-sdk after the issue fixed
// https://github.com/awslabs/aws-sdk-rust/issues/97
pub(crate) mod credential {
    use crate::shared::AwsCredentials;
    use chrono::{DateTime, Utc};
    use serde::Deserialize;
    use std::collections::BTreeMap;

    const AWS_CREDENTIALS_PROVIDER_IP: &str = "169.254.169.254";
    const AWS_CREDENTIALS_PROVIDER_PATH: &str = "latest/meta-data/iam/security-credentials";

    #[derive(Clone, Debug, Deserialize, Default)]
    pub struct AwsCredentialsResponse {
        #[serde(rename = "AccessKeyId")]
        key: String,
        #[serde(rename = "SecretAccessKey")]
        secret: String,
        #[serde(rename = "SessionToken", alias = "Token")]
        token: Option<String>,
        #[serde(rename = "Expiration")]
        expires_at: Option<DateTime<Utc>>,
        #[serde(skip)]
        claims: BTreeMap<String, String>,
    }

    /// Gets the role name to get credentials for using the IAM Metadata Service (169.254.169.254).
    pub fn get_credentials() -> Result<AwsCredentials, String> {
        let role_name_address = format!(
            "http://{}/{}/",
            AWS_CREDENTIALS_PROVIDER_IP, AWS_CREDENTIALS_PROVIDER_PATH
        );
        let role_name = reqwest::blocking::get(role_name_address)
            .unwrap()
            .text()
            .unwrap();
        let credentials_provider_url = format!(
            "http://{}/{}/{}",
            AWS_CREDENTIALS_PROVIDER_IP, AWS_CREDENTIALS_PROVIDER_PATH, role_name
        );
        let credential: AwsCredentialsResponse = reqwest::blocking::get(credentials_provider_url)
            .unwrap()
            .json()
            .unwrap();
        Ok(AwsCredentials {
            aws_key_id: credential.key.clone(),
            aws_secret_key: credential.secret.into(),
            aws_session_token: credential.token.unwrap_or_default(),
        })
    }
}

/// Generates key and encrypts with AWS KMS at the given path
/// TODO: generate in NE after this is merged https://github.com/aws/aws-nitro-enclaves-sdk-c/pull/25
pub fn generate_key(
    path: impl AsRef<Path>,
    region: &str,
    credentials: AwsCredentials,
    kms_key_id: String,
) -> Result<PublicKey, String> {
    let credientials = KmsCredentials::from_keys(
        &credentials.aws_key_id,
        &credentials.aws_secret_key,
        Some(credentials.aws_session_token.clone()),
    );
    let mut csprng = OsRng {};
    let keypair: Keypair = Keypair::generate(&mut csprng);
    let public = keypair.public;
    let privkey_blob = smithy_types::Blob::new(keypair.secret.as_bytes().to_vec());
    let kms_config = Config::builder()
        .region(Region::new(region.to_string()))
        .credentials_provider(credientials)
        .build();

    let client = KmsClient::from_conf(kms_config);
    let generator = client
        .encrypt()
        .set_plaintext(Some(privkey_blob))
        .set_key_id(Some(kms_key_id));

    let rt = Runtime::new().map_err(|err| format!("Failed to init tokio runtime: {}", err))?;
    let output = rt
        .block_on(generator.send())
        .map_err(|e| format!("send to mks to encrypt error: {:?}", e))?;
    // TODO: remove unwrap
    let ciphertext = output.ciphertext_blob.unwrap().into_inner();
    OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .mode(0o600)
        .open(path.as_ref())
        .and_then(|mut file| file.write_all(&*ciphertext))
        .map_err(|e| format!("couldn't write `{}`: {}", path.as_ref().display(), e))?;
    Ok(public)
}
