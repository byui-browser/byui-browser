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

#[test]
fn math_members_are_available_to_page_scripts() {
    let realm = Realm::new();
    let result = realm.evaluate_script("[Math.atan2(1, 1), Math.hypot(3, 4), Math.acos(1), Math.acosh(1), Math.asinh(0), Math.atanh(0), Math.cbrt(27), Math.cosh(0), Math.fround(1.337), Math.imul(-1, 5), Math.log10(100), Math.expm1(0), Math.log1p(0), Math.sinh(0), Math.LOG10E, Math.LOG2E, Math.random() >= 0 && Math.random() < 1]").unwrap();
    assert_eq!(
        result.to_string(),
        "0.7853981633974483,5,0,0,0,0,3,1,1.3370000123977661,-5,2,0,0,0,0.4342944819032518,1.4426950408889634,true"
    );
}
