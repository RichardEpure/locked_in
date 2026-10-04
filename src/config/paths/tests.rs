use super::*;

#[test]
fn relative_override_is_resolved_to_an_absolute_startup_path() {
    let paths = resolve_override(PathBuf::from("relative-data-root")).unwrap();

    assert!(paths.data_root().is_absolute());
    assert!(paths.data_root().ends_with("relative-data-root"));
}

#[test]
fn every_artifact_path_has_the_same_data_root_identity() {
    let paths = ApplicationPaths::from_data_root(PathBuf::from("one-root"));

    assert_eq!(paths.config_path().parent(), Some(paths.data_root()));
    assert_eq!(paths.log_directory().parent(), Some(paths.data_root()));
    assert_eq!(paths.panic_log_path().parent(), Some(paths.data_root()));
    assert_eq!(
        paths.webview_data_directory().parent(),
        Some(paths.data_root())
    );
}
