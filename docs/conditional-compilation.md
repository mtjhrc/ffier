# Conditional compilation

Methods in an exported impl or trait can use ordinary Rust `#[cfg(...)]`
attributes. The metadata retains every annotated method, including methods
disabled on the machine producing the schema.

For an entire exported item, use `#[ffier::export(cfg = "predicate")]`.
Rust removes an item with an outer `#[cfg]` before invoking its export macro;
passing the predicate to `export` lets ffier retain its metadata while gating
the Rust implementation. Gate its `library_definition!` entry with the same
predicate when necessary.

The JSON schema stores predicates as objects, never Rust source strings:

```json
{"cfg": {"target_os": "windows"}}
{"cfg": {"unix": true}}
{"cfg": {"all": [{"unix": true}, {"feature": "files"}]}}
{"cfg": {"not": {"any": [{"windows": true}, {"target_os": "macos"}]}}}
```

Each predicate object has one key. A flag has value `true`; a key/value
predicate has a string value. `all` and `any` contain arrays of predicates;
`not` contains one predicate. Empty `all` is true and empty `any` is false.
An absent `cfg` field means unconditional availability. This representation
also supports custom cfg names without changing the schema's Rust types.
