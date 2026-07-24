use std::fs;
use std::path::PathBuf;

use kube::CustomResourceExt;
use proteus_crd::{ProteusBackup, ProteusRepository, ProteusRestore};

fn main() {
    let out_dir = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("deploy/kustomize/crds"));

    if let Err(err) = fs::create_dir_all(&out_dir) {
        eprintln!("create_dir_all {}: {err}", out_dir.display());
        std::process::exit(1);
    }

    let docs = [
        ("proteusrepositories.yaml", ProteusRepository::crd()),
        ("proteusbackups.yaml", ProteusBackup::crd()),
        ("proteusrestores.yaml", ProteusRestore::crd()),
    ];

    for (name, crd) in docs {
        let path = out_dir.join(name);
        match serde_yaml::to_string(&crd) {
            Ok(yaml) => {
                if let Err(err) = fs::write(&path, yaml) {
                    eprintln!("write {}: {err}", path.display());
                    std::process::exit(1);
                }
                println!("wrote {}", path.display());
            }
            Err(err) => {
                eprintln!("serialize {name}: {err}");
                std::process::exit(1);
            }
        }
    }
}
