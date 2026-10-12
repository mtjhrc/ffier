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


def main() -> None:
    parser = argparse.ArgumentParser(description="Validate generated ffier schema")
    parser.add_argument("schema", type=Path)
    args = parser.parse_args()
    check_schema(args.schema)


if __name__ == "__main__":
    main()
