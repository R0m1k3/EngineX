//! Spec du conteneur : instantané (`UPDATE_FIXTURES=1` pour le réécrire).

use std::path::PathBuf;

use enginex_sandbox::{
    HelperSpec, Mounts, Spec,
    spec::{DockerOpts, container_name, container_spec, helper_name, helper_spec},
};

fn spec() -> (Spec, DockerOpts) {
    let data = PathBuf::from("/data");
    let s = Spec {
        session_id: "0123456789abcdef".into(),
        owner: "instance-1".into(),
        preset: "4:1795000000:deadbeef".into(),
        mounts: Mounts {
            work: data.join("workspace/projects/site/topics/front"),
            codex_home: data.join("sessions/0123456789abcdef/codex"),
            run: data.join("sessions/0123456789abcdef/run"),
            attachments: data.join("sessions/0123456789abcdef/attachments"),
            cache: data.join("cache/site"),
        },
        env: vec![
            ("GITHUB_TOKEN".into(), "ghp_x".into()),
            ("ENGINEX_API_KEY".into(), "ex_k".into()),
        ],
    };
    let o = DockerOpts {
        image: "ghcr.io/r0m1k3/enginex-devbox:latest".into(),
        network: "enginex-sandbox".into(),
        host_gateway: false,
        memory: 8 << 30,
        data_dir: data,
        host_data: "/mnt/user/appdata/enginex".into(),
    };
    (s, o)
}

#[test]
fn container_spec_snapshot() {
    let (s, o) = spec();
    let body = serde_json::to_value(container_spec(&s, &o).unwrap()).unwrap();
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/container-spec.json");
    let text = serde_json::to_string_pretty(&body).unwrap() + "\n";
    if std::env::var("UPDATE_FIXTURES").as_deref() == Ok("1") {
        std::fs::write(&path, &text).unwrap();
    }
    let want = std::fs::read_to_string(&path).expect("instantané absent : UPDATE_FIXTURES=1");
    assert_eq!(text, want, "spec changée : relire puis UPDATE_FIXTURES=1");
    let hc = &body["HostConfig"];
    assert_eq!(hc["Binds"].as_array().unwrap().len(), 5);
    assert_eq!(hc["Binds"][4], "/mnt/user/appdata/enginex/cache/site:/cache:rw");
    assert_eq!(hc["CapDrop"], serde_json::json!(["ALL"]));
    assert_eq!(body["User"], "1000:1000");
    assert!(hc.get("PortBindings").is_none() && body.get("ExposedPorts").is_none());
    assert_eq!(container_name(&s.session_id), "enginex-sbx-01234567");
}

#[test]
fn paths_outside_the_data_dir_are_refused() {
    let (mut s, o) = spec();
    s.mounts.work = "/etc".into();
    assert!(container_spec(&s, &o).is_err());
}

#[test]
fn helper_spec_snapshot() {
    let (_, o) = spec();
    let h = HelperSpec {
        session_id: "0123456789abcdef".into(),
        owner: "instance-1".into(),
        binds: vec![
            (
                PathBuf::from("/data/workspace/projects/site/topics/front"),
                "WORK",
                true,
            ),
            (PathBuf::from("/data/snapshots/topic-7.git"), "SNAP", true),
        ],
        script: "git --version",
        env: vec![("REF".into(), "refs/enginex/x/pre".into())],
        network: false,
    };
    let body = serde_json::to_value(helper_spec(&h, &o).unwrap()).unwrap();
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/helper-spec.json");
    let text = serde_json::to_string_pretty(&body).unwrap() + "\n";
    if std::env::var("UPDATE_FIXTURES").as_deref() == Ok("1") {
        std::fs::write(&path, &text).unwrap();
    }
    let want = std::fs::read_to_string(&path).expect("instantané absent : UPDATE_FIXTURES=1");
    assert_eq!(text, want, "spec changée : relire puis UPDATE_FIXTURES=1");
    let hc = &body["HostConfig"];
    assert_eq!(hc["NetworkMode"], "none");
    assert_eq!(body["NetworkDisabled"], true);
    assert_eq!(
        hc["Binds"].as_array().unwrap().len(),
        2,
        "seuls les deux montages demandés"
    );
    assert_eq!(
        body["Env"],
        serde_json::json!(["REF=refs/enginex/x/pre", "WORK=/mnt/work", "SNAP=/mnt/snap"]),
        "aucune autre variable"
    );
    assert_eq!(hc["CapDrop"], serde_json::json!(["ALL"]));
    assert_eq!(hc["SecurityOpt"], serde_json::json!(["no-new-privileges:true"]));
    assert_eq!(body["User"], "1000:1000");
    assert!(hc.get("PortBindings").is_none() && body.get("ExposedPorts").is_none());
    assert!(body["Labels"].get("enginex.session").is_none());
    assert_eq!(helper_name("0123456789abcdef", "a1b2"), "enginex-helper-01234567-a1b2");
    // Réseau demandé (fetch / push de la page Git) : celui des sandboxes.
    let net = HelperSpec { network: true, ..h };
    let body = serde_json::to_value(helper_spec(&net, &o).unwrap()).unwrap();
    assert_eq!(body["HostConfig"]["NetworkMode"], o.network.as_str());
    assert_eq!(body["NetworkDisabled"], false);
}
