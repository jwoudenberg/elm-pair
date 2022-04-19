use crate::editors;
use crate::lib::log;
use crate::Error;
use std::io::Write;
use std::path::PathBuf;
use std::time::SystemTime;

pub enum License {
    NonCommercial,
    _Commercial {
        order_id: u16,
        expires_at: SystemTime,
    },
}

pub fn read_license() -> License {
    // TODO: implement this
    License::NonCommercial
}

pub fn license_active(license: &License) -> bool {
    match license {
        License::NonCommercial => false,
        License::_Commercial { expires_at, .. } => {
            let now = SystemTime::now();
            expires_at > &now
        }
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
    let path = crate::elm_pair_dir()?.join("license.txt");
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
        License::_Commercial { .. } => include_str!("license_expired_info.txt"),
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
