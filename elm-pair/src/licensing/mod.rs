use crate::editors;
use crate::lib::log;
use crate::Error;
use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;
use std::time::SystemTime;

#[derive(Debug, PartialEq)]
pub enum License {
    NonCommercial,
    Commercial {
        order_id: String,
        expires_at: SystemTime,
    },
}

pub fn read_license() -> License {
    read_license_err().unwrap_or_else(|err| {
        log::error!("failed to read license: {:?}", err);
        License::NonCommercial
    })
}

fn read_license_err() -> Result<License, Error> {
    let license_path = license_path()?;
    if !license_path.exists() {
        Ok(License::NonCommercial)
    } else {
        let key = std::fs::read_to_string(license_path).map_err(|err| {
            log::mk_err!("failed to read license file: {:?}", err)
        })?;
        parse_license(&key)
    }
}

pub fn validate_license(key: &str) -> Result<License, Error> {
    let license = parse_license(key)?;
    let license_path = license_path()?;
    std::fs::write(license_path, key).map_err(|err| {
        log::mk_err!("failed to write license file: {:?}", err)
    })?;
    Ok(license)
}

const YEAR_IN_SECS: u64 = 365 * 24 * 60 * 60;
const ED25519_PUBLIC_KEY: &[u8; 32] = include_bytes!("ed25519.public");

fn parse_license(key: &str) -> Result<License, Error> {
    let key_segments: Vec<&str> = key.split('-').collect();
    match key_segments.as_slice() {
        [] | ["", ..] => Err(log::mk_err!("can't read license version")),
        ["1", order_id, ordered_at_str, signature_base64] => {
            verify_signature(
                &format!("1-{}-{}", order_id, ordered_at_str),
                signature_base64,
            )?;
            let ordered_at_seconds = ordered_at_str.parse::<u64>().unwrap();
            let expires_at_seconds = ordered_at_seconds + YEAR_IN_SECS;
            let expires_at =
                std::time::UNIX_EPOCH + Duration::from_secs(expires_at_seconds);
            Ok(License::Commercial {
                order_id: (*order_id).to_owned(),
                expires_at,
            })
        }
        ["1", ..] => Err(log::mk_err!("v1 license has unexpected structure")),
        [version, ..] => {
            Err(log::mk_err!("license with unknown version {}", version))
        }
    }
}

fn verify_signature(msg: &str, signature_base64: &str) -> Result<(), Error> {
    let public_key = ed25519_compact::PublicKey::new(*ED25519_PUBLIC_KEY);
    let signature_bytes = base64::decode(signature_base64).map_err(|err| {
        log::mk_err!("failed to base64-decode license signature: {:?}", err)
    })?;
    let signature = ed25519_compact::Signature::from_slice(&signature_bytes)
        .map_err(|err| {
            log::mk_err!("failed to read ed25519 license signature: {:?}", err)
        })?;
    public_key.verify(msg, &signature).map_err(|err| {
        log::mk_err!("failed to verify license signature: {:?}", err)
    })
}

#[cfg(test)]
mod parse_license_tests {
    use super::*;

    #[test]
    fn parse_license_with_valid_signature() {
        assert_eq!(
            parse_license("1-88-1334910171-FshObAa93Ua1UzXeoJy53Yk9RivssP2MUAgphKbi21E27TQzzpH+9OaTZDwyzTscWxxXgYmB3LqBlWgpI6AXCQ=="),
            Ok(License::Commercial {
                order_id: "88".to_owned(),
                expires_at: std::time::UNIX_EPOCH
                    + Duration::from_secs(1366446171)
            })
        );
    }

    #[test]
    fn error_for_license_with_too_short_a_signature() {
        let err = parse_license("1-88-1334910171-shortsig").unwrap_err();
        assert!(err.0.contains("failed to read ed25519 license signature"));
    }

    #[test]
    fn error_for_license_with_invalid_signature() {
        let err = parse_license("1-88-1334910171-EshObAa93Ua1UzXeoJy53Yk9RivssP2MUAgphKbi21E27TQzzpH+9OaTZDwyzTscWxxXgYmB3LqBlWgpI6AXCQ==").unwrap_err();
        assert!(err.0.contains("failed to verify license signature"));
    }

    #[test]
    fn cannot_parse_empty_license() {
        let err = parse_license("").unwrap_err();
        assert!(err.0.contains("can't read license version"));
    }

    #[test]
    fn cannot_parse_license_without_segments() {
        let err = parse_license("---").unwrap_err();
        assert!(err.0.contains("can't read license version"));
    }

    #[test]
    fn cannot_parse_license_with_unknown_version() {
        let err = parse_license("62-hi").unwrap_err();
        assert!(err.0.contains("license with unknown version 62"));
    }

    #[test]
    fn cannot_parse_license_with_wrong_number_of_segments() {
        let err = parse_license("1-hi-there").unwrap_err();
        assert!(err.0.contains("v1 license has unexpected structure"));
    }
}

fn license_path() -> Result<PathBuf, Error> {
    let mut dir = dirs::config_dir().unwrap_or_else(|| PathBuf::from("/tmp"));
    dir.push("elm-pair");
    std::fs::create_dir_all(&dir).map_err(|err| {
        log::mk_err!("error while creating directory {:?}: {:?}", dir, err)
    })?;
    dir.push("LICENSE");
    Ok(dir)
}

pub fn license_active(license: &License) -> bool {
    match license {
        License::NonCommercial => false,
        License::Commercial { expires_at, .. } => {
            let now = SystemTime::now();
            expires_at > &now
        }
    }
}

#[cfg(test)]
mod license_active_tests {
    use super::*;

    #[test]
    fn non_commercial_license_is_inactive() {
        assert!(!license_active(&License::NonCommercial));
    }

    #[test]
    fn commercial_license_with_future_expire_time_is_active() {
        let future_time = SystemTime::now() + Duration::from_secs(100);
        let active_license = License::Commercial {
            order_id: "123".to_owned(),
            expires_at: future_time,
        };
        assert!(license_active(&active_license));
    }

    #[test]
    fn commercial_license_with_past_expire_time_is_inactive() {
        let past_time = SystemTime::now() - Duration::from_secs(100);
        let expired_license = License::Commercial {
            order_id: "123".to_owned(),
            expires_at: past_time,
        };
        assert!(!license_active(&expired_license));
    }
}

pub fn show_license_info(license: &License, driver: &dyn editors::Driver) {
    if license_active(license) {
        return;
    }

    let info = license_info(license, driver.kind());

    match write_license_info_to_file(info) {
        Ok(info_path) => {
            driver.show_file(&info_path);
        }
        Err(err) => log::error!("{:?}", err),
    }
}

pub fn write_license_info_to_file<'a, I>(contents: I) -> Result<PathBuf, Error>
where
    I: Iterator<Item = &'a str>,
{
    let path = crate::cache_dir()?.join("license.txt");
    let mut file = std::fs::File::create(&path).map_err(|err| {
        log::mk_err!("failed to create license file {:?}: {:?}", path, err)
    })?;
    for chunk in contents {
        file.write_all(chunk.as_bytes()).map_err(|err| {
            log::mk_err!("failed to write license info to file: {:?}", err)
        })?;
    }
    Ok(path)
}

pub fn license_info(
    license: &License,
    editor_kind: editors::Kind,
) -> impl Iterator<Item = &'static str> {
    let licensing_info = match license {
        License::NonCommercial => include_str!("licensing_info.txt"),
        License::Commercial { .. } => include_str!("license_expired_info.txt"),
    };

    let activation_instructions = match editor_kind {
        editors::Kind::Neovim => {
            include_str!("activate_license_neovim.txt")
        }
        editors::Kind::VsCode => {
            include_str!("activate_license_vscode.txt")
        }
    };

    [licensing_info, "\n", activation_instructions].into_iter()
}
