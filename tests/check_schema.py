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

    traits = {trait["name"]: trait for trait in schema["traits"]}
    expected_slots = {"EmptyTrait": 0, "RetiredTrait": 3, "Processor": 2, "Fruit": 4}
    for name, count in expected_slots.items():
        if traits[name]["num_slots"] != count:
            raise AssertionError(f"{name}: expected {count} virtual method slots")
    if any("max_vtable_slot" in trait for trait in traits.values()):
        raise AssertionError("trait schema must store slot counts, not highest indices")

    error = next(item for item in schema["errors"] if item["name"] == "TestError")
    variants = {v["name"]: v for v in error["variants"]}
    assert variants["CustomMessage"]["data"] == {"kind": "fieldless"}
    assert variants["InvalidInput"]["data"] == {"kind": "tuple", "fields": []}
    assert variants["Fatal"]["data"] == {
        "kind": "tuple", "fields": [{"kind": "opaque"}]
    }
    assert variants["OpaqueUnit"]["data"] == {
        "kind": "tuple", "fields": [{"kind": "opaque"}]
    }
    mixed = variants["MixedError"]["data"]
    assert mixed["kind"] == "tuple"
    assert [f["kind"] for f in mixed["fields"]] == ["typed", "opaque", "typed"]
    assert mixed["fields"][0]["type_ref"]["type"] == "i32"
    assert mixed["fields"][2]["type_ref"]["type"] == "u64"
    for name, type_name in [("NotFound", "str"), ("NumericError", "i32")]:
        data = variants[name]["data"]
        assert data["kind"] == "tuple"
        assert len(data["fields"]) == 1
        assert data["fields"][0]["kind"] == "typed"
        assert data["fields"][0]["type_ref"]["type"] == type_name
    assert all("constructible" not in v and "is_unit" not in v for v in error["variants"])

    widget = next(item for item in schema["exported_types"] if item["name"] == "Widget")
    methods = {method["name"]: method for method in widget["methods"]}
    for name in ["view", "view_mut", "replace_view", "name_bytes", "replace_foreign_borrowed"]:
        assert methods[name]["receiver_lifetime"] == "a"
        assert methods[name]["method_lifetimes"] == ["a"]
    assert "receiver_lifetime" not in methods["new"]
    assert "receiver_lifetime" not in methods["name"]
    foreign = next(p for p in methods["replace_foreign_borrowed"]["params"] if p["name"] == "value")
    assert foreign["type_args"] == ["a"]
    assert foreign["ref_kind"] == "mut"
    processor = next(item for item in schema["traits"] if item["name"] == "Processor")
    name = next(method for method in processor["methods"] if method["name"] == "name")
    assert name["receiver_lifetime"] == "a"
    assert name["ret"]["ref_lifetime"] == "a"
    receiver_defaults = next(
        item for item in schema["traits"] if item["name"] == "ReceiverDefaults"
    )
    assert receiver_defaults["num_slots"] == 6
    choose = next(
        method for method in receiver_defaults["methods"] if method["name"] == "choose"
    )
    assert choose["receiver_lifetime"] == "a"
    assert choose["method_lifetimes"] == ["a"]
    choose_elided = next(
        method for method in receiver_defaults["methods"] if method["name"] == "choose_elided"
    )
    assert "receiver_lifetime" not in choose_elided

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


def main() -> None:
    parser = argparse.ArgumentParser(description="Validate generated ffier schema")
    parser.add_argument("schema", type=Path)
    args = parser.parse_args()
    check_schema(args.schema)


if __name__ == "__main__":
    main()
