use js::Realm;

#[test]
fn core_globals_are_available_in_fresh_realms_and_bytecode() {
    let source = "'' + Boolean(0) + ',' + Boolean('x') + ',' + Math.max(2, 7) + ',' + JSON.stringify([1, 'x']) + ',' + isNaN(NaN) + ',' + (Infinity > 1)";
    let realm = Realm::new();
    let interpreted = realm.evaluate_script(source).unwrap();
    let chunk = js::bytecode::compile(&js::parse(source).unwrap()).unwrap();
    let bytecode = js::Vm::new().execute(&chunk).unwrap();
    assert_eq!(interpreted, bytecode);
}

#[test]
fn unsupported_json_parse_reports_a_clear_runtime_diagnostic() {
    let realm = Realm::new();
    let error = realm.evaluate_script("JSON.parse('{}')").unwrap_err();
    assert!(error.message.contains("JSON.parse is not supported yet"));
}
