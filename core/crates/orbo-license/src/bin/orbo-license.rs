//! Issues Orbo Pro license keys. Runs only on the developer's machine.
//!
//!   orbo-license keygen                      new signing key in ~/.orbo/license-signing-key (once!)
//!   orbo-license issue --name N --email E    print a key (1 year of updates)
//!   orbo-license verify <KEY>                check a key against the built-in public key

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use orbo_license::{decode_signing_key, encode_public_key, encode_signing_key, generate_signing_key, issue, verify, License};

fn key_path() -> PathBuf {
    std::env::var_os("ORBO_SIGNING_KEY")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".orbo/license-signing-key"))
}

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

fn today() -> String {
    let out = std::process::Command::new("date").arg("+%Y-%m-%d").output().expect("date");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn plus_years(date: &str, years: i32) -> String {
    let y: i32 = date[..4].parse().unwrap_or(2026);
    format!("{}{}", y + years, &date[4..])
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("keygen") => {
            let path = key_path();
            if path.exists() {
                bail!("{} existiert schon. Ein neuer Schlüssel würde alle ausgegebenen Lizenzen ungültig machen.", path.display());
            }
            std::fs::create_dir_all(path.parent().unwrap())?;
            let sk = generate_signing_key();
            std::fs::write(&path, encode_signing_key(&sk))?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
            }
            println!("Privater Schlüssel: {} (sichern, niemals teilen)", path.display());
            println!("Öffentlicher Schlüssel (in crates/orbo-license/src/public_key.txt eintragen):\n{}", encode_public_key(&sk));
        }
        Some("issue") => {
            let sk = decode_signing_key(&std::fs::read_to_string(key_path()).context("Signierschlüssel fehlt, erst `orbo-license keygen`")?)?;
            let name = arg(&args, "--name").context("--name fehlt")?;
            let email = arg(&args, "--email").context("--email fehlt")?;
            let years: i32 = arg(&args, "--years").and_then(|y| y.parse().ok()).unwrap_or(1);
            let issued = arg(&args, "--issued").unwrap_or_else(today);
            let id = arg(&args, "--id").unwrap_or_else(|| format!("ORB-{}-{}", issued.replace('-', ""), &email.len()));
            let lic = License { v: 1, id, name, email, plan: "pro".into(), updates_until: plus_years(&issued, years), issued };
            println!("{}", issue(&lic, &sk));
        }
        Some("verify") => {
            let key = args.get(1).context("Schlüssel fehlt")?;
            match verify(key) {
                Ok(l) => println!("gültig: {} <{}>, Updates bis {}", l.name, l.email, l.updates_until),
                Err(e) => bail!("{e}"),
            }
        }
        _ => println!("orbo-license keygen | issue --name N --email E [--years 1] | verify KEY"),
    }
    Ok(())
}
