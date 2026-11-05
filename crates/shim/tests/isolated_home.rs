//! MUS-95: no shim test may see the real home. A shim started with it finds
//! the developer's pairing in `~/.config/muster` and connects to their live
//! daemon, as `resume_channel_detect` did on a paired Mac.

mod common;

/// Every test file starts the shim through `common::shim`, which gives it an
/// isolated home. Starting the binary directly would inherit the real one.
#[test]
fn every_shim_test_starts_the_shim_with_an_isolated_home() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let mut direct = Vec::new();
    for entry in std::fs::read_dir(&dir).expect("read tests dir") {
        let path = entry.expect("dir entry").path();
        // This file names the patterns it looks for.
        if path.extension().is_none_or(|e| e != "rs") || path.ends_with("isolated_home.rs") {
            continue;
        }
        let text = std::fs::read_to_string(&path).expect("read test file");
        for (n, line) in text.lines().enumerate() {
            let starts_it = line.contains("Command::new(SHIM)")
                || (line.contains("Command::new(") && line.contains("CARGO_BIN_EXE_muster-shim"));
            if starts_it {
                direct.push(format!("{}:{}", path.display(), n + 1));
            }
        }
    }
    assert!(
        direct.is_empty(),
        "these start muster-shim without common::shim(), so with the real HOME: {direct:?}"
    );
}

#[test]
fn the_isolated_home_isnt_the_real_one() {
    let home = common::isolated_home();
    assert_ne!(Some(home.to_path_buf()), common::real_home());
    assert!(
        !home.join(".config").join("muster").join("pairing").exists(),
        "the isolated home has a pairing in it"
    );
    let cmd = common::shim();
    let env: Vec<_> = cmd.get_envs().collect();
    for var in ["HOME", "XDG_CONFIG_HOME", "USERPROFILE", "APPDATA"] {
        let set = env
            .iter()
            .find(|(k, _)| *k == var)
            .and_then(|(_, v)| *v)
            .unwrap_or_else(|| panic!("common::shim() doesn't set {var}"));
        assert!(
            std::path::Path::new(set).starts_with(home),
            "{var} is {set:?}, outside the isolated home"
        );
    }
}
