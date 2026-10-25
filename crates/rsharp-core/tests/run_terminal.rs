#[test]
fn existing_run_configurations_stay_in_the_run_console_and_the_terminal_choice_survives_storage() {
    use rsharp_core::run::RunConfiguration;
    let mut configuration: RunConfiguration = serde_json::from_str(r#"{"name":"App","project":"App.csproj"}"#).unwrap();
    assert!(!configuration.use_terminal, "existing configurations unexpectedly moved to the terminal");
    configuration.use_terminal = true;
    let stored = serde_json::to_string(&configuration).unwrap();
    assert!(serde_json::from_str::<RunConfiguration>(&stored).unwrap().use_terminal);
}
