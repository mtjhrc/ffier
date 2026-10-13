#!/usr/bin/env python3

import argparse
import json
from pathlib import Path


EXPECTED_NAMES = (
    "OptionalWidget",
    "OptionalWorker",
    "OptionalMode",
    "OptionalFlags",
    "ft_optional_apply",
    "ft_optional_mode_name",
    "ft_optional_merge_flags",
)

SCHEMA_SECTIONS = (
    "exported_types",
    "traits",
    "trait_impls",
    "enum_constants",
    "bitflags_constants",
    "free_functions",
)


def check_schema(schema_path: Path) -> None:
    schema = json.loads(schema_path.read_text())
    schema_text = json.dumps(schema)
    error = next(item for item in schema["errors"] if item["name"] == "TestError")
    assert {v["name"]: v["constructible"] for v in error["variants"]} == {
        "NotFound": False,
        "CustomMessage": True,
        "InvalidInput": True,
        "NumericError": False,
        "Fatal": False,
        "GadgetError": False,
        "DeviceError": False,
        "ContextError": False,
        "FileError": False,
    }
    assert {v["name"]: v["is_unit"] for v in error["variants"]} == {
        "NotFound": False,
        "CustomMessage": True,
        "InvalidInput": False,
        "NumericError": False,
        "Fatal": False,
        "GadgetError": False,
        "DeviceError": False,
        "ContextError": False,
        "FileError": False,
    }
    for variant in error["variants"]:
        factory = variant.get("constructor_ffi_name")
        assert bool(factory) == bool(variant.get("fields")), variant
        if factory:
            assert factory.startswith("ft_test_error_") and factory.endswith("_new")

    missing_names = [name for name in EXPECTED_NAMES if name not in schema_text]
    if missing_names:
        raise AssertionError(
            f"cfg-gated items missing from schema: {missing_names}"
        )

    all_items = [
        item
        for section in SCHEMA_SECTIONS
        for item in schema.get(section, [])
    ]
    cfg_items = [item for item in all_items if item.get("cfg")]
    if len(cfg_items) < 8:
        raise AssertionError(f"expected >=8 cfg-gated items, got {len(cfg_items)}")

    def method_return(section: str, owner: str, method_name: str) -> dict:
        item = next(item for item in schema[section] if item["name"] == owner)
        method = next(method for method in item["methods"] if method["name"] == method_name)
        return method["ret"]

    value_snapshot = {
        "value_structs": schema["value_structs"],
        "event_queue_pop_event": method_return("exported_types", "EventQueue", "pop_event"),
        "input_backend_next_event": method_return("traits", "InputBackend", "next_event"),
    }
    expected = json.loads((Path(__file__).parent / "expected_value_schema.json").read_text())
    if value_snapshot != expected:
        raise AssertionError("value-struct schema differs from expected_value_schema.json")

    def method_param(owner: str, name: str) -> dict:
        item = next(item for item in schema["exported_types"] if item["name"] == owner)
        return next(method for method in item["methods"] if method["name"] == name)["params"][0]

    assert method_param("RetainedFruit", "new_static")["lifetime_bounds"] == ["static"]
    assert method_param("RetainedFruit", "new_dynamic")["lifetime_bounds"] == ["a"]
    snapshot = method_param("View", "snapshot_count")
    assert snapshot["type_args"] == ["a"]
    assert snapshot["lifetime_bounds"] == ["b"]
    assert method_param("RetainedFruit", "borrowed_value")["ref_kind"] == "shared"
    assert method_param("RetainedFruit", "borrowed_value_dynamic")["ref_lifetime"] == "_"
    assert method_param("RetainedFruit", "borrowed_mut_value")["ref_kind"] == "mut"
    retain = next(item for item in schema["free_functions"] if item["name"] == "retain_fruit")
    assert retain["lifetimes"] == ["a"]
    assert retain["ret"]["type_args"] == ["a"]

    def guarantees(section: str, name: str) -> dict:
        item = next(item for item in schema[section] if item["name"] == name)
        return item.get("thread_safety", {})

    assert guarantees("exported_types", "Widget") == {"send": True, "sync": True}
    assert guarantees("exported_types", "View") == {"send": True, "sync": True}
    assert guarantees("exported_types", "SendCounter") == {"send": True}
    assert guarantees("exported_types", "SyncCounter") == {"sync": True}
    assert guarantees("exported_types", "BackendWorker") == {"send": True}
    assert guarantees("exported_types", "Gadget") == {}
    assert guarantees("traits", "SendBackend") == {"send": True}
    assert guarantees("traits", "SyncBackend") == {"sync": True}
    assert guarantees("traits", "ThreadBackend") == {"send": True, "sync": True}
    assert guarantees("traits", "Fruit") == {}
    run_send = next(item for item in schema["free_functions"] if item["name"] == "run_send_backend")
    assert run_send["params"][0]["thread_safety"] == {"send": True}
    assert run_send["params"][0]["lifetime_bounds"] == ["static"]

    attach = next(item for item in schema["traits"] if item["name"] == "AttachDevice")
    assert attach["no_vtable"]
    assert not attach.get("dyn_dispatch", False)
    assert attach["methods"] == []
    assert attach["thread_safety"] == {"send": True}
    assert guarantees("exported_types", "AttachItem") == {"send": True}
    implementation = next(
        item for item in schema["trait_impls"] if item["trait_name"] == "AttachDevice"
    )
    assert implementation["methods"] == []
    assert implementation["trait_lifetime_args"] == ["a"]
    assert "AttachContext" not in schema_text and "Requirements" not in schema_text
    add = next(item for item in schema["free_functions"] if item["name"] == "attach_device")
    assert add["params"][0]["type_args"] == ["a"]
    assert add["params"][0]["lifetime_bounds"] == ["a"]


def main() -> None:
    parser = argparse.ArgumentParser(description="Validate generated ffier schema")
    parser.add_argument("schema", type=Path)
    args = parser.parse_args()
    check_schema(args.schema)


if __name__ == "__main__":
    main()
