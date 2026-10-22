//! Prints the `ProxyKubeApi` CRD on stdout (`task gen:crds`). With
//! `--links <file>`, also writes the docs' CRD IDE links there
//! (`weebo_kit_api::links`): the objects the CRD's fields make (the cluster
//! behind an external service, the OIDC issuers), and the schematic kinds
//! the IDE draws but Kubernetes never sees — the kit's shared ones and this
//! project's `KubeCluster` and `AccessMethod` (`schematic.rs`).

mod schematic;

use std::collections::BTreeMap;

use crd::ProxyKubeApi;
use kube::CustomResourceExt;
use weebo_kit_api::links::{self, GroupKind, Roles};

fn main() -> anyhow::Result<()> {
    let crd = ProxyKubeApi::crd();
    print!("{}", serde_yaml_ng::to_string(&crd)?);
    let mut args = std::env::args().skip(1);
    if let (Some(flag), Some(path)) = (args.next(), args.next()) {
        anyhow::ensure!(flag == "--links", "usage: crdgen [--links <file>]");
        let own = GroupKind::new(&crd.spec.group, &crd.spec.names.kind);
        // No instance kind here: no `InstanceRef` either, the roles name none.
        let roles = Roles {
            instance: own.clone(),
            local_instance: own,
        };
        let kinds = BTreeMap::from([(
            crd.spec.names.kind.clone(),
            links::of::<ProxyKubeApi>(&roles)?,
        )]);
        let mut drawn = weebo_kit_api::schematic::shared(&roles)?;
        drawn.extend(schematic::own(&roles)?);
        let file = serde_json::json!({ "version": 1, "kinds": kinds, "schematic": drawn });
        std::fs::write(&path, serde_json::to_string_pretty(&file)? + "\n")?;
    }
    Ok(())
}
