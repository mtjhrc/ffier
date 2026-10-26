use ffier_gen_c_header::{Options, generate};
use ffier_schema::{Cfg, Library};
use serde_json::{Value, json};

fn schema() -> Value {
    json!({
        "prefix": "ft",
        "type_registry": {
            "Result": {"kind": {"primitive": {"c_type": "FtResult"}}, "bless": "result"},
            "str": {"kind": {"string": {"c_name": "FtStr"}}, "bless": "str"},
            "Object": {"kind": {"handle": {"c_name": "FtObject"}}, "bless": "object"},
            "VtableHandle": {
                "kind": {"handle": {"c_name": "FtVtableHandle"}}, "bless": "vtable_handle"
            },
            "Widget": {"kind": {"handle": {"c_name": "FtWidget"}}},
            "Processor": {"kind": {"trait": {"c_name": "FtProcessor"}}, "type_tag": 2}
        },
        "exported_types": [], "errors": [], "traits": [], "trait_impls": []
    })
}

fn header(schema: Value) -> String {
    let library: Library = serde_json::from_value(schema).unwrap();
    generate(&library, "TEST_H", &Options { fn_typedefs: true }).unwrap()
}

fn method(name: &str, cfg: Option<Cfg>, index: Option<usize>) -> Value {
    let mut method = json!({
        "name": name, "ffi_name": format!("ft_processor_{name}"),
        "receiver": "ref", "params": [], "ret": {"return_kind": "void"}, "cfg": cfg
    });
    if let Some(index) = index {
        method["trait_definition"] = json!({"index": index, "has_default": false});
    }
    method
}

fn processor(methods: Vec<Value>, num_slots: usize) -> Value {
    json!({
        "name": "Processor", "destroy_ffi_name": "ft_processor_destroy",
        "type_tag_constant": "FT_PROCESSOR_TYPE_TAG",
        "vtable_struct_c_name": "FtProcessorVtable", "wrapper_c_name": "FtVtableProcessor",
        "vtable_struct_name": "ProcessorVtable", "wrapper_name": "VtableProcessor",
        "own_method_count": methods.len(), "methods": methods, "num_slots": num_slots
    })
}

#[test]
fn conditional_sections_and_methods_use_library_guards() {
    let mut schema = schema();
    schema["primitives_prefix"] = json!("shared");
    schema["exported_types"] = json!([{
        "name": "Widget", "destroy_ffi_name": "ft_widget_destroy", "is_builder_type": false,
        "cfg": Cfg::Feature("owner".into()),
        "methods": [method("widget_run", Some(Cfg::Feature("files".into())), None)]
    }]);
    schema["traits"] = json!([processor(
        vec![method(
            "process",
            Some(Cfg::Feature("files".into())),
            Some(0)
        )],
        1
    )]);
    schema["traits"][0]["cfg"] = json!(Cfg::Feature("callbacks".into()));
    schema["trait_impls"] = json!([{
        "trait_name": "Processor", "struct_name": "Widget",
        "cfg": Cfg::All(vec![Cfg::Feature("owner".into()), Cfg::Feature("callbacks".into())]),
        "methods": [method("widget_process", Some(Cfg::Feature("files".into())), None)]
    }]);
    schema["enum_constants"] = json!([{
        "name": "Mode", "variants": [{"name": "On", "c_name": "FT_MODE_ON", "value": 1}],
        "cfg": Cfg::Feature("modes".into())
    }]);
    schema["bitflags_constants"] = json!([{
        "name": "Flags", "variants": [{"name": "Read", "c_name": "FT_FLAGS_READ", "value": 1}],
        "cfg": Cfg::Feature("flags".into())
    }]);
    schema["free_functions"] = json!([{
        "name": "open", "ffi_name": "ft_open", "params": [], "ret": {"return_kind": "void"},
        "cfg": Cfg::Feature("files".into())
    }]);
    let header = header(schema);
    assert!(header.contains("typedef void* FtWidget;"));
    assert!(header.contains("typedef void* FtProcessor;"));
    assert!(header.contains("#if defined(FT_CFG_FEATURE_FILES)\nvoid ft_processor_widget_run(FtWidget handle);\ntypedef void (*ft_processor_widget_run_fn)(FtWidget handle);\n#endif"));
    assert!(header.contains("#if defined(FT_CFG_FEATURE_MODES)"));
    assert!(header.contains("#if defined(FT_CFG_FEATURE_FLAGS)"));
    assert!(!header.contains("SHARED_CFG_"));

    #[cfg(unix)]
    {
        let disabled = c_compiler(&header, &["-E", "-P"]);
        for name in [
            "ft_widget_destroy(",
            "ft_processor_destroy(",
            "ft_open(",
            "FT_MODE_ON",
            "FT_FLAGS_READ",
        ] {
            assert!(
                !disabled.contains(name),
                "disabled declaration leaked: {name}"
            );
        }
        let source =
            format!("{header}\nvoid check(void) {{ ft_open_fn open = ft_open; (void)open; }}\n");
        c_compiler(
            &source,
            &[
                "-fsyntax-only",
                "-DFT_CFG_FEATURE_OWNER",
                "-DFT_CFG_FEATURE_CALLBACKS",
                "-DFT_CFG_FEATURE_FILES",
                "-DFT_CFG_FEATURE_MODES",
                "-DFT_CFG_FEATURE_FLAGS",
            ],
        );
    }
}

#[test]
fn conditional_owners_keep_handles_used_by_unconditional_signatures() {
    let mut schema = schema();
    schema["exported_types"] = json!([{
        "name": "Widget", "destroy_ffi_name": "ft_widget_destroy", "is_builder_type": false,
        "cfg": Cfg::Feature("owner".into()),
        "methods": [method("run", None, None)]
    }]);
    schema["traits"] = json!([processor(vec![method("process", None, Some(0))], 1)]);
    schema["traits"][0]["cfg"] = json!(Cfg::Feature("owner".into()));
    schema["free_functions"] = json!([{
        "name": "inspect", "ffi_name": "ft_inspect",
        "params": [
            {"name": "widget", "param_kind": "regular", "type": "Widget", "ref_kind": "shared"},
            {"name": "processor", "param_kind": "impl_trait", "trait_name": "Processor"}
        ],
        "ret": {"return_kind": "value", "type": "Widget"}
    }]);
    let header = header(schema);
    assert!(header.contains("FtWidget ft_inspect(FtWidget widget, FtProcessor processor);"));
    #[cfg(unix)]
    {
        let source = format!(
            "{header}\nvoid check(FtWidget widget, FtProcessor processor) {{\n    ft_inspect_fn inspect = ft_inspect;\n    (void)inspect(widget, processor);\n}}\n"
        );
        c_compiler(&source, &["-fsyntax-only"]);
        c_compiler(&source, &["-fsyntax-only", "-DFT_CFG_FEATURE_OWNER"]);
    }
}

#[test]
fn invalid_or_colliding_guard_names_return_errors() {
    for (predicates, expected) in [
        (
            vec![
                Cfg::Feature("foo-bar".into()),
                Cfg::Feature("foo_bar".into()),
            ],
            "FT_CFG_FEATURE_FOO_BAR",
        ),
        (
            vec![
                Cfg::Feature("foo_bar".into()),
                Cfg::Feature("FOO_BAR".into()),
            ],
            "FT_CFG_FEATURE_FOO_BAR",
        ),
        (
            vec![
                Cfg::TargetOs("custom-os".into()),
                Cfg::TargetOs("custom_os".into()),
            ],
            "FT_CFG_TARGET_OS_CUSTOM_OS",
        ),
        (vec![Cfg::Feature("".into())], "unsupported cfg name"),
        (
            vec![Cfg::Feature("with spaces".into())],
            "unsupported cfg name",
        ),
        (
            vec![Cfg::Feature("unicode-část".into())],
            "unsupported cfg name",
        ),
        (
            vec![Cfg::TargetOs("os/name".into())],
            "unsupported cfg name",
        ),
    ] {
        let mut schema = schema();
        schema["free_functions"] = json!(
            predicates
                .into_iter()
                .enumerate()
                .map(|(index, cfg)| {
                    json!({
                        "name": format!("call_{index}"), "ffi_name": format!("ft_call_{index}"),
                        "params": [], "ret": {"return_kind": "void"},
                        "cfg": Cfg::All(vec![Cfg::Not(Box::new(cfg))])
                    })
                })
                .collect::<Vec<_>>()
        );
        let library: Library = serde_json::from_value(schema).unwrap();
        assert!(
            generate(&library, "TEST_H", &Options::default())
                .unwrap_err()
                .contains(expected)
        );
    }

    let mut owner_schema = schema();
    owner_schema["exported_types"] = json!([{
        "name": "Widget", "destroy_ffi_name": "ft_widget_destroy", "is_builder_type": false,
        "cfg": Cfg::Feature("foo-bar".into()),
        "methods": [method("run", Some(Cfg::Feature("foo_bar".into())), None)]
    }]);
    let library: Library = serde_json::from_value(owner_schema).unwrap();
    assert!(
        generate(&library, "TEST_H", &Options::default())
            .unwrap_err()
            .contains("FT_CFG_FEATURE_FOO_BAR")
    );

    let mut schema = schema();
    schema["free_functions"] = json!([{
        "name": "with_hyphen", "ffi_name": "ft_with_hyphen", "params": [], "ret": {"return_kind": "void"},
        "cfg": Cfg::Feature("foo-bar".into())
    }]);
    let header = header(schema);
    assert!(header.contains("#if defined(FT_CFG_FEATURE_FOO_BAR)\nvoid ft_with_hyphen(void);"));
}

#[test]
fn constant_predicates_remove_guards_and_disabled_declarations() {
    let mut schema = schema();
    let feature = Cfg::Feature("files".into());
    schema["free_functions"] = json!([
        {"name": "always", "ffi_name": "ft_always", "params": [], "ret": {"return_kind": "void"},
         "cfg": Cfg::Any(vec![feature.clone(), Cfg::Not(Box::new(feature.clone()))])},
        {"name": "never", "ffi_name": "ft_never", "params": [], "ret": {"return_kind": "void"},
         "cfg": Cfg::All(vec![feature.clone(), Cfg::Not(Box::new(feature.clone()))])},
        {"name": "conditional", "ffi_name": "ft_conditional", "params": [], "ret": {"return_kind": "void"},
         "cfg": Cfg::All(vec![Cfg::All(vec![]), feature])}
    ]);
    let header = header(schema);
    assert!(header.contains("void ft_always(void);"));
    assert!(!header.contains("ft_never"));
    assert!(header.contains("#if defined(FT_CFG_FEATURE_FILES)\nvoid ft_conditional(void);"));
    assert_eq!(header.matches("FT_CFG_FEATURE_FILES").count(), 1);
    assert!(!header.contains("#if 0\n"));
    assert!(!header.contains("#if 1\n"));
}

#[test]
fn conditional_vtable_slots_keep_reserved_gaps() {
    let mut schema = schema();
    schema["traits"] = json!([processor(
        vec![
            method("optional", Some(Cfg::Feature("files".into())), Some(0)),
            method("never", Some(Cfg::Any(vec![])), Some(2)),
            method("last", None, Some(3)),
        ],
        4
    )]);
    let header = header(schema);
    assert!(header.contains("(*__reserved_1)(void)"));
    assert!(header.contains("(*__reserved_2)(void)"));
    assert!(!header.contains("(*never)"));
    assert!(!header.contains("#if 0\n"));
    assert!(!header.contains("#if 1\n"));

    #[cfg(unix)]
    for (defines, first) in [
        (vec![], "__reserved_0"),
        (vec!["-DFT_CFG_FEATURE_FILES"], "optional"),
    ] {
        let source = format!(
            "{header}\n_Static_assert(FT_VTABLE_LEN(FtProcessorVtable) == 4, \"slot count\");\n_Static_assert(offsetof(FtProcessorVtable, {first}) == offsetof(FtProcessorVtable, drop) + sizeof(void (*)(void)), \"first slot\");\n_Static_assert(offsetof(FtProcessorVtable, last) == offsetof(FtProcessorVtable, drop) + 4 * sizeof(void (*)(void)), \"last slot\");\n"
        );
        let mut arguments = vec!["-fsyntax-only"];
        arguments.extend(defines);
        c_compiler(&source, &arguments);
    }
}

#[cfg(unix)]
fn c_compiler(source: &str, arguments: &[&str]) -> String {
    use std::{
        io::Write,
        process::{Command, Stdio},
    };
    let mut child = Command::new("cc")
        .args(["-std=c11", "-Werror", "-x", "c", "-"])
        .args(arguments)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("C compiler");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(source.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}
