#![recursion_limit = "512"]
#![cfg(unix)]

use std::os::unix::io::{AsRawFd, BorrowedFd, OwnedFd};

use ffier_test_foreign_lib_via_cdylib::ForeignItem;

// ---------------------------------------------------------------------------
// Error type — uses thiserror for Display/Error, ffier for FFI codes
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
#[cfg_attr(feature = "ffi", derive(ffier::FfiError))]
#[non_exhaustive]
pub enum TestError {
    #[error("not found: {0}")]
    #[cfg_attr(feature = "ffi", ffier(code = 1))]
    #[non_exhaustive]
    NotFound(Box<str>),
    #[error("custom error message")]
    #[cfg_attr(feature = "ffi", ffier(code = 2, message = "custom error message"))]
    CustomMessage,
    #[error("invalid input")]
    #[cfg_attr(feature = "ffi", ffier(code = 3, message = "invalid input"))]
    InvalidInput(),
    #[error("numeric error: {0}")]
    #[cfg_attr(feature = "ffi", ffier(code = 4))]
    NumericError(i32),
    /// An unrecoverable error carrying an arbitrary `anyhow` error chain.
    /// The inner value is Rust-only and not marshalled across FFI (`opaque`).
    #[error(transparent)]
    #[cfg_attr(feature = "ffi", ffier(code = 5, opaque))]
    Fatal(anyhow::Error),
    #[error("mixed error: {0}, {1}, {2}")]
    #[cfg_attr(feature = "ffi", ffier(code = 6))]
    MixedError(
        i32,
        #[cfg_attr(feature = "ffi", ffier(opaque))] anyhow::Error,
        u64,
    ),
    #[error("opaque unit payload")]
    #[cfg_attr(feature = "ffi", ffier(code = 7, opaque))]
    OpaqueUnit(()),
}

thread_local! {
    static TEST_ERROR_DROPS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

impl Drop for TestError {
    fn drop(&mut self) {
        TEST_ERROR_DROPS.with(|drops| drops.set(drops.get() + 1));
    }
}

/// Test instrumentation: native errors destroyed on the current thread.
#[cfg_attr(feature = "ffi", ffier::export)]
pub fn test_error_drop_count() -> usize {
    TEST_ERROR_DROPS.with(std::cell::Cell::get)
}

/// Exercise error forwarding with each callback return convention.
#[cfg_attr(feature = "ffi", ffier::export)]
pub trait ErrorForwarder {
    #[cfg_attr(feature = "ffi", ffier(index = 0))]
    fn forward_value(&self) -> Result<i32, TestError>;

    #[cfg_attr(feature = "ffi", ffier(index = 1))]
    fn forward_handle(&self) -> Result<Gadget, TestError>;

    #[cfg_attr(feature = "ffi", ffier(index = 2))]
    fn forward_unit(&self) -> Result<(), TestError>;
}

/// Reconstruct a callback error as its native type, then return it to the client.
#[cfg_attr(feature = "ffi", ffier::export)]
pub fn roundtrip_callback_error(
    callback: impl ErrorForwarder,
    method: u32,
) -> Result<(), TestError> {
    let error = match method {
        0 => callback.forward_value().unwrap_err(),
        1 => callback.forward_handle().err().expect("callback must fail"),
        2 => callback.forward_unit().unwrap_err(),
        _ => panic!("unknown callback method"),
    };
    // Inspect the native enum, not merely the client wrapper or its type tag.
    match &error {
        TestError::CustomMessage => assert_eq!(error.to_string(), "custom error message"),
        TestError::InvalidInput() => assert_eq!(error.to_string(), "invalid input"),
        TestError::NumericError(value) => {
            assert_eq!(*value, 76);
            assert_eq!(error.to_string(), "numeric error: 76");
        }
        TestError::NotFound(value) => assert_eq!(&**value, "error"),
        TestError::Fatal(error) => {
            assert_eq!(error.to_string(), "opaque callback error");
            assert_eq!(error.root_cause().to_string(), "hidden detail");
        }
        TestError::MixedError(first, hidden, last) => {
            assert_eq!((*first, *last), (17, 900));
            assert_eq!(hidden.to_string(), "hidden detail");
        }
        TestError::OpaqueUnit(()) => {}
    }
    Err(error)
}

// ---------------------------------------------------------------------------
// Widget — primary type exercising most param/return patterns
// ---------------------------------------------------------------------------

pub struct Widget {
    count: i32,
    name: String,
    active: bool,
    tags: String,
    gadget: Gadget,
    gadgets: Vec<Gadget>,
}

#[cfg_attr(feature = "ffi", ffier::export)]
impl Widget {
    /// Create a new widget with default values.
    pub fn new() -> Self {
        Widget {
            count: 0,
            name: String::from("widget"),
            active: true,
            tags: String::new(),
            gadget: Gadget { value: 42 },
            gadgets: vec![Gadget { value: 10 }, Gadget { value: 20 }],
        }
    }

    /// Create a widget with a given name.
    pub fn with_name(name: &str) -> Self {
        Widget {
            count: 0,
            name: name.to_owned(),
            gadgets: vec![Gadget { value: 10 }, Gadget { value: 20 }],
            active: true,
            tags: String::new(),
            gadget: Gadget { value: 42 },
        }
    }

    #[cfg(windows)]
    pub fn windows_noop(&self) {}

    #[cfg(feature = "optional-entry")]
    pub fn optional_noop(&self) {}

    /// Get the current count.
    pub fn get_count(&self) -> i32 {
        self.count
    }

    /// Set the count.
    pub fn set_count(&mut self, n: i32) {
        self.count = n;
    }

    /// Set count and return `&mut Self` for method chaining.
    pub fn with_count(&mut self, n: i32) -> &mut Self {
        self.count = n;
        self
    }

    /// Get the widget name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Get the raw name bytes.
    pub fn data(&self) -> &[u8] {
        self.name.as_bytes()
    }

    /// Sum the bytes of a byte slice.
    pub fn sum_bytes(&self, data: &[u8]) -> i32 {
        data.iter().map(|&b| b as i32).sum()
    }

    /// Echo back the given string (zero-copy borrow passthrough).
    pub fn echo<'a>(&self, s: &'a str) -> &'a str {
        s
    }

    /// Check if the widget is active.
    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Negate a 64-bit integer.
    pub fn negate(&self, v: i64) -> i64 {
        -v
    }

    pub fn echo_f32(&self, value: f32) -> f32 {
        value
    }

    pub fn echo_f64(&self, value: f64) -> f64 {
        value
    }

    pub fn echo_isize(&self, value: isize) -> isize {
        value
    }

    /// Validate internal state (always succeeds for default widget).
    pub fn validate(&self) -> Result<(), TestError> {
        if self.count >= 0 {
            Ok(())
        } else {
            Err(TestError::InvalidInput())
        }
    }

    /// Parse a count value from the name length, returning error if name matches trigger.
    ///
    /// # Arguments
    ///
    /// - `s`: the input string whose length becomes the count.
    ///
    /// # Returns
    ///
    /// The count derived from the name length.
    pub fn parse_count(&self, s: &str) -> Result<i32, TestError> {
        if s == "error" {
            Err(TestError::NotFound(s.into()))
        } else {
            Ok(s.len() as i32)
        }
    }

    /// Describe a code as a string.
    ///
    /// # Arguments
    ///
    /// * `code` - the numeric code to look up.
    pub fn describe(&self, code: i32) -> Result<&str, TestError> {
        match code {
            0 => Ok("zero"),
            1 => Ok("one"),
            _ => Err(TestError::NotFound(format!("code {code}").into())),
        }
    }

    /// Always fails with an error.
    pub fn fail_always(&self) -> Result<(), TestError> {
        Err(TestError::CustomMessage)
    }

    /// Always fails with an error (value variant).
    pub fn fail_with_value(&self) -> Result<i32, TestError> {
        Err(TestError::InvalidInput())
    }

    /// Always fails with a numeric error carrying an i32 payload.
    pub fn fail_with_number(&self, n: i32) -> Result<(), TestError> {
        Err(TestError::NumericError(n))
    }

    /// Fail with typed fields around an opaque payload field.
    pub fn fail_with_mixed(&self) -> Result<(), TestError> {
        Err(TestError::MixedError(
            17,
            anyhow::anyhow!("hidden detail"),
            900,
        ))
    }

    /// Fail with opaque fields, including a zero-sized native payload.
    pub fn fail_with_opaque(&self, unit_payload: bool) -> Result<(), TestError> {
        if unit_payload {
            Err(TestError::OpaqueUnit(()))
        } else {
            Err(TestError::Fatal(
                anyhow::anyhow!("hidden detail").context("opaque callback error"),
            ))
        }
    }

    /// Set tags from a string slice.
    pub fn set_tags(&mut self, tags: &[&str]) {
        self.tags = tags.join(",");
    }

    /// Get joined tags.
    pub fn tags_joined(&self) -> &str {
        &self.tags
    }

    /// Create a new gadget with the widget's count as initial value.
    pub fn create_gadget(&self) -> Gadget {
        Gadget { value: self.count }
    }

    /// Return a borrowed reference to the widget's internal gadget.
    pub fn gadget(&self) -> &Gadget {
        &self.gadget
    }

    /// Return a borrowed slice of the widget's gadgets (&[T] pattern).
    pub fn gadgets(&self) -> &[Gadget] {
        &self.gadgets
    }

    /// Try to create a gadget; fails if ok is false.
    pub fn try_create_gadget(&self, ok: bool) -> Result<Gadget, TestError> {
        if ok {
            Ok(Gadget { value: self.count })
        } else {
            Err(TestError::NotFound("gadget creation failed".into()))
        }
    }

    /// Read a gadget's value.
    pub fn read_gadget(&self, g: &Gadget) -> i32 {
        g.value
    }

    /// Update a gadget's value.
    pub fn update_gadget(&self, g: &mut Gadget, v: i32) {
        g.value = v;
    }

    /// Set the name, or reset to default if `None`.
    pub fn set_name(&mut self, name: Option<&str>) {
        self.name = name.unwrap_or("widget").to_owned();
    }

    /// Get an owned copy of the name.
    pub fn owned_name(&self) -> Box<str> {
        self.name.clone().into_boxed_str()
    }

    /// Add a permission flag to the widget's permissions and return the result.
    pub fn add_permission(&self, base: Permissions, flag: Permissions) -> Permissions {
        base | flag
    }

    /// Sum the values of a slice of gadgets.
    pub fn sum_gadgets(&self, gadgets: &[&Gadget]) -> i32 {
        gadgets.iter().map(|g| g.value).sum()
    }

    /// Sum a slice of u32 values.
    pub fn sum_values(&self, values: &[u32]) -> u32 {
        values.iter().sum()
    }

    /// Consume the widget (by-value self, void return).
    pub fn consume(self) {
        drop(self);
    }

    /// Get the raw fd number from a borrowed fd.
    pub fn fd_number(&self, fd: BorrowedFd<'_>) -> i32 {
        fd.as_raw_fd()
    }

    /// Get the raw fd number, or -1 if None.
    pub fn fd_number_optional(&self, fd: Option<BorrowedFd<'_>>) -> i32 {
        fd.map_or(-1, |f| f.as_raw_fd())
    }

    /// Maybe return a borrowed fd depending on `selector`:
    /// < 0 → error, 0 → Ok(None), > 0 → Ok(Some(stdin)).
    pub fn maybe_fd(&self, selector: i32) -> Result<Option<BorrowedFd<'_>>, TestError> {
        if selector < 0 {
            Err(TestError::InvalidInput())
        } else if selector == 0 {
            Ok(None)
        } else {
            // Safety: fd 0 (stdin) exists for the process lifetime
            Ok(Some(unsafe { BorrowedFd::borrow_raw(0) }))
        }
    }

    /// Duplicate a file descriptor (returns owned fd).
    pub fn dup_fd(&self, fd: BorrowedFd<'_>) -> OwnedFd {
        fd.try_clone_to_owned().expect("dup failed")
    }

    /// Maybe duplicate a file descriptor; returns None if input is None.
    pub fn maybe_dup_fd(&self, fd: Option<BorrowedFd<'_>>) -> Result<Option<OwnedFd>, TestError> {
        match fd {
            Some(f) => f
                .try_clone_to_owned()
                .map(Some)
                .map_err(|_| TestError::InvalidInput()),
            None => Ok(None),
        }
    }

    /// Optionally return an owned fd depending on `selector`:
    /// < 0 → error, 0 → Ok(None), > 0 → Ok(Some(dup(stdin))).
    pub fn maybe_owned_fd(&self, selector: i32) -> Result<Option<OwnedFd>, TestError> {
        if selector < 0 {
            Err(TestError::InvalidInput())
        } else if selector == 0 {
            Ok(None)
        } else {
            // Safety: fd 0 (stdin) exists for the process lifetime
            let stdin = unsafe { BorrowedFd::borrow_raw(0) };
            stdin
                .try_clone_to_owned()
                .map(Some)
                .map_err(|_| TestError::InvalidInput())
        }
    }

    /// Apply a foreign config to this widget (tests foreign param on a method).
    pub fn apply_config(
        &mut self,
        #[cfg_attr(feature = "ffi", ffier(foreign = ::ffier_test_foreign_lib_via_cdylib, c_name = "FlForeignConfig"))]
        config: &ffier_test_foreign_lib_via_cdylib::ForeignConfig,
    ) {
        self.name = config.name().to_owned();
        self.count = config.value();
    }

    /// Read a lifetime-bearing foreign handle.
    pub fn read_foreign_borrowed<'a>(
        &self,
        #[cfg_attr(feature = "ffi", ffier(foreign = ::ffier_test_foreign_lib_via_cdylib, c_name = "FlForeignBorrowed"))]
        value: &ffier_test_foreign_lib_via_cdylib::ForeignBorrowed<'a>,
    ) -> i32 {
        value.value().len() as i32
    }
}

impl Default for Widget {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Gadget — second handle type (used as param/return in Widget methods)
// ---------------------------------------------------------------------------

pub struct Gadget {
    value: i32,
}

#[cfg_attr(feature = "ffi", ffier::export)]
impl Gadget {
    /// Get the gadget value.
    pub fn get(&self) -> i32 {
        self.value
    }
}

// ---------------------------------------------------------------------------
// Config — builder pattern (by-value self -> Self)
// ---------------------------------------------------------------------------

pub struct Config {
    name: String,
    size: i32,
}

#[cfg_attr(feature = "ffi", ffier::export)]
impl Config {
    /// Create a new config.
    pub fn new() -> Self {
        Config {
            name: String::new(),
            size: 0,
        }
    }

    /// Set the name (builder pattern: consumes self, returns Self).
    pub fn set_name(mut self, name: &str) -> Self {
        self.name = name.to_owned();
        self
    }

    /// Set the size (builder pattern).
    pub fn set_size(mut self, size: i32) -> Self {
        self.size = size;
        self
    }

    /// Validate and return self, or error if name is empty.
    pub fn validated(self) -> Result<Self, TestError> {
        if self.name.is_empty() {
            Err(TestError::InvalidInput())
        } else {
            Ok(self)
        }
    }

    /// Get the config name.
    pub fn get_name(&self) -> &str {
        &self.name
    }

    /// Get the config size.
    pub fn get_size(&self) -> i32 {
        self.size
    }
}

impl Default for Config {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// GizmoBuilder + Gizmo — builder consuming self returning different type
// ---------------------------------------------------------------------------

pub struct Gizmo {
    name: String,
    size: i32,
}

#[cfg_attr(feature = "ffi", ffier::export)]
impl Gizmo {
    /// Get the gizmo name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Get the gizmo size.
    pub fn size(&self) -> i32 {
        self.size
    }
}

pub struct GizmoBuilder {
    name: String,
    size: i32,
}

#[cfg_attr(feature = "ffi", ffier::export)]
impl GizmoBuilder {
    /// Create a new gizmo builder.
    pub fn new() -> Self {
        GizmoBuilder {
            name: String::new(),
            size: 0,
        }
    }

    /// Set the gizmo name.
    pub fn set_name(&mut self, name: &str) {
        self.name = name.to_owned();
    }

    /// Set the gizmo size.
    pub fn set_size(&mut self, size: i32) {
        self.size = size;
    }

    /// Build the gizmo (consumes builder, returns different type).
    pub fn build(self) -> Gizmo {
        Gizmo {
            name: self.name,
            size: self.size,
        }
    }

    /// Try to build the gizmo; fails if name is empty.
    pub fn try_build(self) -> Result<Gizmo, TestError> {
        if self.name.is_empty() {
            Err(TestError::InvalidInput())
        } else {
            Ok(Gizmo {
                name: self.name,
                size: self.size,
            })
        }
    }
}

impl Default for GizmoBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// View<'a> — lifetime-parameterized type borrowing another handle
// ---------------------------------------------------------------------------

pub struct View<'a> {
    source: &'a Widget,
    label: String,
}

#[cfg_attr(feature = "ffi", ffier::export)]
impl<'a> View<'a> {
    /// Create a view that borrows a widget.
    pub fn create(source: &'a Widget) -> Self {
        View {
            source,
            label: String::from("default"),
        }
    }

    /// Create a view with a custom label.
    ///
    /// Takes two reference params so lifetime elision can't resolve `'_`
    /// in the return type — the struct lifetime must be preserved explicitly.
    pub fn create_labeled(source: &'a Widget, label: &str) -> Self {
        View {
            source,
            label: label.to_owned(),
        }
    }

    /// Read the source widget's count through the borrow.
    pub fn source_count(&self) -> i32 {
        self.source.count
    }

    /// Set the view label.
    pub fn set_label(&mut self, label: &str) {
        self.label = label.to_owned();
    }

    /// Get the view label.
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Copy label from another snapshot (tests impl Trait auto-dispatch).
    pub fn copy_label(&mut self, other: impl Snapshot<'a>) {
        self.label = other.snap_description().to_owned();
    }
}

// ---------------------------------------------------------------------------
// ViewFactory — non-lifetime struct returning lifetime-parameterized type
// Tests method-level lifetime introduction when multiple input refs exist.
// ---------------------------------------------------------------------------

pub struct ViewFactory;

#[cfg_attr(feature = "ffi", ffier::export)]
impl ViewFactory {
    pub fn new() -> Self {
        ViewFactory
    }

    /// Create a view from a source widget with a label.
    ///
    /// Multiple reference params + lifetime-parameterized return type forces
    /// the generator to introduce a method-level lifetime (can't elide).
    pub fn create_view<'a>(source: &'a Widget, label: &str) -> View<'a> {
        View {
            source,
            label: label.to_owned(),
        }
    }
}

impl Default for ViewFactory {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Implementable trait: Processor
// ---------------------------------------------------------------------------

#[cfg_attr(feature = "ffi", ffier::export)]
pub trait Processor {
    #[cfg_attr(feature = "ffi", ffier(index = 0))]
    fn process(&self, input: i32) -> i32;
    #[cfg_attr(feature = "ffi", ffier(index = 1))]
    fn name(&self) -> &str;
}

/// Empty traits have no virtual method slots, only the fixed drop callback.
#[cfg_attr(feature = "ffi", ffier::export)]
pub trait EmptyTrait {}

/// Retired slots still count even when no virtual methods remain.
#[cfg_attr(feature = "ffi", ffier::export(reserved(0, 2)))]
pub trait RetiredTrait {}

// ---------------------------------------------------------------------------
// Pipeline — uses dyn_param with Processor
// ---------------------------------------------------------------------------

pub struct Pipeline {
    results: Vec<i32>,
}

#[cfg_attr(feature = "ffi", ffier::export)]
impl Pipeline {
    /// Create a new pipeline.
    pub fn new() -> Self {
        Pipeline {
            results: Vec::new(),
        }
    }

    /// Run a processor on the given input.
    pub fn run(&mut self, proc: impl Processor, input: i32) {
        let result = proc.process(input);
        self.results.push(result);
    }

    /// Get the number of results.
    pub fn result_count(&self) -> i32 {
        self.results.len() as i32
    }

    /// Get the last result, or error if empty.
    pub fn last_result(&self) -> Result<i32, TestError> {
        self.results
            .last()
            .copied()
            .ok_or_else(|| TestError::NotFound("no results".into()))
    }
}

impl Default for Pipeline {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Mixer — uses dyn_param with multiple CONCRETE types (not vtable)
// Tests that ffier-gen-rust generates correct IntoXxxHandle impls for each.
// ---------------------------------------------------------------------------

pub struct Apple {
    weight: i32,
}

#[cfg_attr(feature = "ffi", ffier::export)]
impl Apple {
    pub fn new(weight: i32) -> Self {
        Apple { weight }
    }
}

pub struct Orange {
    juice: i32,
}

#[cfg_attr(feature = "ffi", ffier::export)]
impl Orange {
    pub fn new(juice: i32) -> Self {
        Orange { juice }
    }
}

#[cfg(feature = "fruit-label")]
#[cfg_attr(feature = "ffi", ffier::export)]
pub trait Fruit {
    #[cfg_attr(feature = "ffi", ffier(index = 0))]
    fn value(&self) -> i32;

    /// Default label — returns "fruit". C can override by providing the
    /// vtable field; if left NULL, this default runs.
    #[cfg_attr(feature = "ffi", ffier(index = 1))]
    fn label(&self) -> &str {
        "fruit"
    }

    /// Fallible count — returns error if input is negative.
    #[cfg_attr(feature = "ffi", ffier(index = 2))]
    fn try_count(&self, input: i32) -> Result<i32, TestError>;

    /// Count how many of the given tags match the fruit's label.
    #[cfg_attr(feature = "ffi", ffier(index = 3))]
    fn count_tags(&self, tags: &[&str]) -> i32;
}

#[cfg(not(feature = "fruit-label"))]
#[cfg_attr(feature = "ffi", ffier::export)]
pub trait Fruit {
    #[cfg_attr(feature = "ffi", ffier(index = 0))]
    fn value(&self) -> i32;

    #[cfg_attr(feature = "ffi", ffier(index = 2))]
    fn try_count(&self, input: i32) -> Result<i32, TestError>;

    #[cfg_attr(feature = "ffi", ffier(index = 3))]
    fn count_tags(&self, tags: &[&str]) -> i32;
}

#[cfg_attr(feature = "ffi", ffier::export)]
impl Fruit for Apple {
    fn value(&self) -> i32 {
        self.weight
    }
    fn try_count(&self, input: i32) -> Result<i32, TestError> {
        if input < 0 {
            Err(TestError::InvalidInput())
        } else {
            Ok(self.weight + input)
        }
    }
    fn count_tags(&self, tags: &[&str]) -> i32 {
        tags.len() as i32 + self.weight
    }
}

#[cfg_attr(feature = "ffi", ffier::export)]
impl Fruit for Orange {
    fn value(&self) -> i32 {
        self.juice
    }
    fn try_count(&self, input: i32) -> Result<i32, TestError> {
        if input < 0 {
            Err(TestError::InvalidInput())
        } else {
            Ok(self.juice * input)
        }
    }
    fn count_tags(&self, tags: &[&str]) -> i32 {
        tags.len() as i32 + self.juice
    }
}

// Extra fruit types so that blend(a: impl Fruit, b: impl Fruit) has
// 9 variants (8 concrete + VtableFruit). 9^2 = 81 > 64 dispatch limit.
pub struct Banana(i32);
#[cfg_attr(feature = "ffi", ffier::export)]
impl Banana {
    pub fn new(v: i32) -> Self {
        Banana(v)
    }
}
#[cfg_attr(feature = "ffi", ffier::export)]
impl Fruit for Banana {
    fn value(&self) -> i32 {
        self.0
    }
    fn try_count(&self, input: i32) -> Result<i32, TestError> {
        if input < 0 {
            Err(TestError::InvalidInput())
        } else {
            Ok(self.0 + input)
        }
    }
    fn count_tags(&self, tags: &[&str]) -> i32 {
        tags.len() as i32
    }
}

pub struct Mango(i32);
#[cfg_attr(feature = "ffi", ffier::export)]
impl Mango {
    pub fn new(v: i32) -> Self {
        Mango(v)
    }
}
#[cfg_attr(feature = "ffi", ffier::export)]
impl Fruit for Mango {
    fn value(&self) -> i32 {
        self.0
    }
    fn try_count(&self, input: i32) -> Result<i32, TestError> {
        if input < 0 {
            Err(TestError::InvalidInput())
        } else {
            Ok(self.0 + input)
        }
    }
    fn count_tags(&self, tags: &[&str]) -> i32 {
        tags.len() as i32
    }
}

pub struct Peach(i32);
#[cfg_attr(feature = "ffi", ffier::export)]
impl Peach {
    pub fn new(v: i32) -> Self {
        Peach(v)
    }
}
#[cfg_attr(feature = "ffi", ffier::export)]
impl Fruit for Peach {
    fn value(&self) -> i32 {
        self.0
    }
    fn try_count(&self, input: i32) -> Result<i32, TestError> {
        if input < 0 {
            Err(TestError::InvalidInput())
        } else {
            Ok(self.0 + input)
        }
    }
    fn count_tags(&self, tags: &[&str]) -> i32 {
        tags.len() as i32
    }
}

pub struct Plum(i32);
#[cfg_attr(feature = "ffi", ffier::export)]
impl Plum {
    pub fn new(v: i32) -> Self {
        Plum(v)
    }
}
#[cfg_attr(feature = "ffi", ffier::export)]
impl Fruit for Plum {
    fn value(&self) -> i32 {
        self.0
    }
    fn try_count(&self, input: i32) -> Result<i32, TestError> {
        if input < 0 {
            Err(TestError::InvalidInput())
        } else {
            Ok(self.0 + input)
        }
    }
    fn count_tags(&self, tags: &[&str]) -> i32 {
        tags.len() as i32
    }
}

pub struct Grape(i32);
#[cfg_attr(feature = "ffi", ffier::export)]
impl Grape {
    pub fn new(v: i32) -> Self {
        Grape(v)
    }
}
#[cfg_attr(feature = "ffi", ffier::export)]
impl Fruit for Grape {
    fn value(&self) -> i32 {
        self.0
    }
    fn try_count(&self, input: i32) -> Result<i32, TestError> {
        if input < 0 {
            Err(TestError::InvalidInput())
        } else {
            Ok(self.0 + input)
        }
    }
    fn count_tags(&self, tags: &[&str]) -> i32 {
        tags.len() as i32
    }
}

pub struct Lemon(i32);
#[cfg_attr(feature = "ffi", ffier::export)]
impl Lemon {
    pub fn new(v: i32) -> Self {
        Lemon(v)
    }
}
#[cfg_attr(feature = "ffi", ffier::export)]
impl Fruit for Lemon {
    fn value(&self) -> i32 {
        self.0
    }
    fn try_count(&self, input: i32) -> Result<i32, TestError> {
        if input < 0 {
            Err(TestError::InvalidInput())
        } else {
            Ok(self.0 + input)
        }
    }
    fn count_tags(&self, tags: &[&str]) -> i32 {
        tags.len() as i32
    }
}

pub struct Mixer {
    total: i32,
}

#[cfg_attr(feature = "ffi", ffier::export)]
#[allow(clippy::should_implement_trait)]
impl Mixer {
    pub fn new() -> Self {
        Mixer { total: 0 }
    }

    pub fn add(mut self, fruit: impl Fruit) -> Self {
        self.total += fruit.value();
        self
    }

    /// Returns the length of a fruit's label. Used to test that vtable
    /// default method detection works for custom client types crossing FFI.
    #[cfg(feature = "fruit-label")]
    pub fn fruit_label_len(&self, fruit: impl Fruit) -> i32 {
        fruit.label().len() as i32
    }

    /// Both concrete (9^2=81 > 64, override with annotation).
    pub fn blend_concrete(
        &mut self,
        #[cfg_attr(feature = "ffi", ffier(dispatch = concrete))] a: impl Fruit,
        #[cfg_attr(feature = "ffi", ffier(dispatch = concrete))] b: impl Fruit,
    ) -> i32 {
        let sum = a.value() + b.value();
        self.total += sum;
        sum
    }

    /// First concrete, second vtable (hybrid: 9+9=18 branches).
    pub fn blend_hybrid(
        &mut self,
        a: impl Fruit,
        #[cfg_attr(feature = "ffi", ffier(dispatch = vtable))] b: impl Fruit,
    ) -> i32 {
        let sum = a.value() + b.value();
        self.total += sum;
        sum
    }

    /// Both vtable (9+9=18 branches).
    pub fn blend_dynamic(
        &mut self,
        #[cfg_attr(feature = "ffi", ffier(dispatch = vtable))] a: impl Fruit,
        #[cfg_attr(feature = "ffi", ffier(dispatch = vtable))] b: impl Fruit,
    ) -> i32 {
        let sum = a.value() + b.value();
        self.total += sum;
        sum
    }

    pub fn total(&self) -> i32 {
        self.total
    }
}

impl Default for Mixer {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Marker trait with supertrait — tests no_vtable (C can't implement, no
// VtableCategorizable generated, but concrete dispatch still works)
// ---------------------------------------------------------------------------

/// A marker trait: implementors are `Fruit`s that can be categorized.
/// Has no methods of its own — used purely as a type bound.
#[cfg_attr(feature = "ffi", ffier::export(no_vtable))]
pub trait Categorizable: Fruit {}

#[cfg_attr(feature = "ffi", ffier::export)]
impl Categorizable for Apple {}

#[cfg_attr(feature = "ffi", ffier::export)]
impl Categorizable for Orange {}

/// Accepts only categorizable fruits (not all fruits).
#[cfg_attr(feature = "ffi", ffier::export)]
pub fn categorizable_value(fruit: impl Categorizable) -> i32 {
    fruit.value()
}

// ---------------------------------------------------------------------------
// Trait with non-FFI-able methods — tests #[ffier(skip)]
// ---------------------------------------------------------------------------

/// A type that can't cross FFI (not FfiType, not a handle).
pub struct InternalState {
    _data: Vec<u8>,
}

pub trait Attachment {
    fn label(&self) -> &str;
    /// Internal method with non-FFI-able params.
    fn attach(&self, state: &InternalState) -> bool;
}

pub struct Sprocket {
    name: String,
}

#[cfg_attr(feature = "ffi", ffier::export)]
impl Sprocket {
    pub fn new(name: &str) -> Self {
        Sprocket {
            name: name.to_owned(),
        }
    }
}

#[cfg_attr(feature = "ffi", ffier::export)]
impl Attachment for Sprocket {
    fn label(&self) -> &str {
        &self.name
    }

    #[cfg_attr(feature = "ffi", ffier(skip))]
    fn attach(&self, state: &InternalState) -> bool {
        !state._data.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Lifetime-parameterized trait impl — tests that `impl<'a> Trait<'a> for Struct<'a>`
// preserves lifetimes in generated client code.
// ---------------------------------------------------------------------------

pub trait Snapshot<'a> {
    fn snap_description(&self) -> &str;
    fn snap_source_count(&self) -> i32;
}

#[cfg_attr(feature = "ffi", ffier::export)]
impl<'a> Snapshot<'a> for View<'a> {
    fn snap_description(&self) -> &str {
        &self.label
    }

    fn snap_source_count(&self) -> i32 {
        self.source.count
    }
}

/// Static impl — tests that `impl Trait<'static> for Struct` preserves
/// the concrete `'static` lifetime on the trait.
#[cfg_attr(feature = "ffi", ffier::export)]
impl Snapshot<'static> for Widget {
    fn snap_description(&self) -> &str {
        &self.name
    }

    fn snap_source_count(&self) -> i32 {
        self.count
    }
}

/// Generic lifetime impl for a struct without lifetime params — tests that
/// `impl<'a> Trait<'a> for Struct` does NOT add a spurious `<'a>` to the struct
/// in the generated client code.
#[cfg_attr(feature = "ffi", ffier::export)]
impl<'a> Snapshot<'a> for Gadget {
    fn snap_description(&self) -> &str {
        "gadget"
    }

    fn snap_source_count(&self) -> i32 {
        self.value
    }
}

// ---------------------------------------------------------------------------
// Foreign trait — tests `extern trait` for traits from external crates
// ---------------------------------------------------------------------------

pub use foreign_trait_crate::Weighable;

#[cfg(feature = "ffi")]
#[ffier::export(foreign)]
trait Weighable {
    #[ffier(index = 0)]
    fn weight_grams(&self) -> i32;
}

#[cfg_attr(feature = "ffi", ffier::export)]
impl Weighable for Apple {
    fn weight_grams(&self) -> i32 {
        self.weight * 10
    }
}

// ---------------------------------------------------------------------------
// Fixed-layout value structs — passed inline or by pointer, no handles
// ---------------------------------------------------------------------------

/// Nested value struct: inner payload used as an inline field.
#[cfg_attr(feature = "ffi", ffier::export)]
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EventPayload {
    pub code: u16,
    pub weight: f32,
}

/// Value struct containing scalars, an enum, bitflags, and a nested struct.
#[cfg_attr(feature = "ffi", ffier::export)]
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InputEvent {
    pub level: LogLevel,
    pub permissions: Permissions,
    pub sequence: u32,
    pub active: bool,
    pub payload: EventPayload,
}

#[cfg_attr(feature = "ffi", ffier::export)]
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Off = 0,
    Error = 1,
    Warn = 2,
    Info = 3,
    Debug = 4,
    Trace = 5,
}

// ---------------------------------------------------------------------------
// Bitflags — flags type exported as C #define constants
// ---------------------------------------------------------------------------

ffier::export_bitflags! {
    bitflags::bitflags! {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub struct Permissions: u32 {
            const READ    = 0b0001;
            const WRITE   = 0b0010;
            const EXECUTE = 0b0100;
            const DELETE  = 0b1000;
        }
    }
}

// ---------------------------------------------------------------------------
// Free functions — not methods on any type
// ---------------------------------------------------------------------------

#[cfg_attr(feature = "ffi", ffier::export)]
/// Describe a log level as a string.
pub fn log_level_name(level: LogLevel) -> &'static str {
    match level {
        LogLevel::Off => "off",
        LogLevel::Error => "error",
        LogLevel::Warn => "warn",
        LogLevel::Info => "info",
        LogLevel::Debug => "debug",
        LogLevel::Trace => "trace",
    }
}

#[cfg_attr(feature = "ffi", ffier::export)]
/// Check if a log level is enabled (everything above Off).
pub fn log_level_is_enabled(level: LogLevel) -> bool {
    level as u32 > 0
}

#[cfg_attr(feature = "ffi", ffier::export)]
/// Count the number of gadgets in a slice and return the sum of their values.
pub fn sum_gadget_values(gadgets: &[&Gadget]) -> i32 {
    gadgets.iter().map(|g| g.value).sum()
}

#[cfg_attr(feature = "ffi", ffier::export)]
/// Read a value struct passed inline.
pub fn event_sequence(event: InputEvent) -> u32 {
    event.sequence
}

#[cfg_attr(feature = "ffi", ffier::export)]
/// Read a value struct through a shared pointer.
pub fn event_payload_code(event: &InputEvent) -> u16 {
    event.payload.code
}

#[cfg_attr(feature = "ffi", ffier::export)]
/// Mutate a value struct through a mutable pointer.
pub fn event_bump_payload_code(event: &mut InputEvent) {
    event.payload.code += 1;
}

#[cfg_attr(feature = "ffi", ffier::export)]
/// Return a value struct by value.
pub fn event_make(level: LogLevel, sequence: u32) -> InputEvent {
    InputEvent {
        level,
        permissions: Permissions::empty(),
        sequence,
        active: true,
        payload: EventPayload {
            code: 0,
            weight: 0.0,
        },
    }
}

#[cfg_attr(feature = "ffi", ffier::export)]
/// Return a value struct inside a Result.
pub fn event_checked(sequence: u32) -> Result<EventPayload, TestError> {
    if sequence == 0 {
        return Err(TestError::InvalidInput());
    }
    Ok(EventPayload {
        code: sequence as u16,
        weight: 1.5,
    })
}

/// Callback that yields optional value structs — exercises
/// `Result<Option<ValueStruct>, E>` through a vtable slot.
#[cfg_attr(feature = "ffi", ffier::export)]
pub trait SampleSource {
    #[cfg_attr(feature = "ffi", ffier(index = 0))]
    fn next_sample(&mut self) -> Result<Option<InputEvent>, TestError>;
}

/// Concrete SampleSource: yields one sample, then None, then errors.
pub struct FixedSource {
    step: u8,
}

#[cfg_attr(feature = "ffi", ffier::export)]
impl FixedSource {
    pub fn new() -> Self {
        Self { step: 0 }
    }

    /// Same optional value shape as the vtable slot, but as a plain
    /// exported method on a handle.
    pub fn peek_sample(&mut self) -> Result<Option<InputEvent>, TestError> {
        <Self as SampleSource>::next_sample(self)
    }
}

impl Default for FixedSource {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg_attr(feature = "ffi", ffier::export)]
impl SampleSource for FixedSource {
    fn next_sample(&mut self) -> Result<Option<InputEvent>, TestError> {
        let step = self.step;
        self.step += 1;
        match step {
            0 => Ok(Some(InputEvent {
                level: LogLevel::Warn,
                permissions: Permissions::empty(),
                sequence: 7,
                active: true,
                payload: EventPayload {
                    code: 11,
                    weight: 2.5,
                },
            })),
            1 => Ok(None),
            _ => Err(TestError::InvalidInput()),
        }
    }
}

/// Read one sample's sequence from a callback, defaulting to 0 when absent.
#[cfg_attr(feature = "ffi", ffier::export)]
pub fn source_next_sequence(mut source: impl SampleSource) -> Result<u32, TestError> {
    Ok(source.next_sample()?.map_or(0, |sample| sample.sequence))
}

/// Free function with the same optional-value return shape, so the generated
/// client's free-function path is exercised too.
#[cfg_attr(feature = "ffi", ffier::export)]
pub fn sample_at(selector: u32) -> Result<Option<InputEvent>, TestError> {
    match selector {
        0 => Ok(Some(InputEvent {
            level: LogLevel::Info,
            permissions: Permissions::empty(),
            sequence: 3,
            active: false,
            payload: EventPayload {
                code: 5,
                weight: 1.0,
            },
        })),
        1 => Ok(None),
        _ => Err(TestError::InvalidInput()),
    }
}

#[cfg_attr(feature = "ffi", ffier::export)]
/// Duplicate a file descriptor.
pub fn clone_fd(fd: BorrowedFd<'_>) -> Result<OwnedFd, TestError> {
    fd.try_clone_to_owned()
        .map_err(|_| TestError::InvalidInput())
}

pub mod optional_api {
    // The cfg predicate is passed via ffier::export(cfg = "...") so the proc
    // macro emits metadata unconditionally while gating the generated code.
    // When ffi is off, #[cfg_attr(not(feature = "ffi"), cfg(...))] gates the
    // item directly.

    #[cfg_attr(feature = "ffi", ffier::export(cfg = "feature = \"optional-entry\""))]
    #[cfg_attr(not(feature = "ffi"), cfg(feature = "optional-entry"))]
    pub trait OptionalWorker {
        #[cfg_attr(feature = "ffi", ffier(index = 0))]
        fn amplify(&self, input: i32) -> i32;
    }

    #[cfg(feature = "optional-entry")]
    pub struct OptionalWidget {
        base: i32,
    }

    #[cfg_attr(feature = "ffi", ffier::export(cfg = "feature = \"optional-entry\""))]
    #[cfg_attr(not(feature = "ffi"), cfg(feature = "optional-entry"))]
    impl OptionalWidget {
        pub fn new(base: i32) -> Self {
            Self { base }
        }

        pub fn base(&self) -> i32 {
            self.base
        }
    }

    #[cfg(feature = "optional-entry")]
    impl Default for OptionalWidget {
        fn default() -> Self {
            Self::new(0)
        }
    }

    #[cfg_attr(feature = "ffi", ffier::export(cfg = "feature = \"optional-entry\""))]
    #[cfg_attr(not(feature = "ffi"), cfg(feature = "optional-entry"))]
    impl OptionalWorker for OptionalWidget {
        fn amplify(&self, input: i32) -> i32 {
            self.base * input
        }
    }

    #[cfg_attr(feature = "ffi", ffier::export(cfg = "feature = \"optional-entry\""))]
    #[cfg_attr(not(feature = "ffi"), cfg(feature = "optional-entry"))]
    #[repr(u32)]
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum OptionalMode {
        Basic = 1,
        Advanced = 2,
    }

    ffier::export_bitflags! {
        #[cfg(feature = "optional-entry")]
        bitflags::bitflags! {
            #[derive(Debug, Clone, Copy, PartialEq, Eq)]
            pub struct OptionalFlags: u32 {
                const READ  = 0b0001;
                const WRITE = 0b0010;
            }
        }
    }

    #[cfg_attr(feature = "ffi", ffier::export(cfg = "feature = \"optional-entry\""))]
    #[cfg_attr(not(feature = "ffi"), cfg(feature = "optional-entry"))]
    pub fn optional_apply(worker: impl OptionalWorker, input: i32) -> i32 {
        worker.amplify(input)
    }

    #[cfg_attr(feature = "ffi", ffier::export(cfg = "feature = \"optional-entry\""))]
    #[cfg_attr(not(feature = "ffi"), cfg(feature = "optional-entry"))]
    pub fn optional_mode_name(mode: OptionalMode) -> &'static str {
        match mode {
            OptionalMode::Basic => "basic",
            OptionalMode::Advanced => "advanced",
        }
    }

    #[cfg_attr(feature = "ffi", ffier::export(cfg = "feature = \"optional-entry\""))]
    #[cfg_attr(not(feature = "ffi"), cfg(feature = "optional-entry"))]
    pub fn optional_merge_flags(a: OptionalFlags, b: OptionalFlags) -> OptionalFlags {
        a | b
    }

    /// Value struct gated behind a Cargo feature — covers cfg on value structs.
    #[cfg_attr(feature = "ffi", ffier::export(cfg = "feature = \"optional-entry\""))]
    #[cfg_attr(not(feature = "ffi"), cfg(feature = "optional-entry"))]
    #[repr(C)]
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct OptionalSample {
        pub mode: OptionalMode,
        pub flags: OptionalFlags,
    }

    #[cfg_attr(feature = "ffi", ffier::export(cfg = "feature = \"optional-entry\""))]
    #[cfg_attr(not(feature = "ffi"), cfg(feature = "optional-entry"))]
    pub fn optional_sample(sample: OptionalSample) -> u32 {
        sample.mode as u32 + sample.flags.bits()
    }
}

#[cfg(feature = "optional-entry")]
pub use optional_api::{
    OptionalFlags, OptionalMode, OptionalSample, OptionalWidget, OptionalWorker, optional_apply,
    optional_merge_flags, optional_mode_name, optional_sample,
};

// ---------------------------------------------------------------------------
// Foreign type tests — accept/return types from another ffier library via its
// generated client bindings (opaque handles + accessor methods, no field access).
// ---------------------------------------------------------------------------

/// Apply a foreign config: extract the name and value, set them on the widget.
#[cfg_attr(feature = "ffi", ffier::export)]
pub fn apply_foreign_config(
    widget: &mut Widget,
    #[cfg_attr(feature = "ffi", ffier(foreign = ::ffier_test_foreign_lib_via_cdylib, c_name = "FlForeignConfig"))]
    config: &ffier_test_foreign_lib_via_cdylib::ForeignConfig,
) {
    widget.name = config.name().to_owned();
    widget.count = config.value();
}

/// Read a foreign item's score.
#[cfg_attr(feature = "ffi", ffier::export)]
pub fn read_foreign_item_score(
    #[cfg_attr(feature = "ffi", ffier(foreign = ::ffier_test_foreign_lib_via_cdylib, c_name = "FlForeignItem"))]
    item: &ForeignItem,
) -> i32 {
    item.score()
}

/// Create a foreign item from our library's data (tests foreign return type).
#[cfg_attr(feature = "ffi", ffier::export)]
#[cfg_attr(feature = "ffi", ffier(foreign_return = ::ffier_test_foreign_lib_via_cdylib, c_name = "FlForeignItem"))]
pub fn create_foreign_item(
    label: &str,
    score: i32,
) -> ffier_test_foreign_lib_via_cdylib::ForeignItem {
    ffier_test_foreign_lib_via_cdylib::ForeignItem::new(label, score)
}

/// Mutate a foreign item's score (tests &mut foreign param).
#[cfg_attr(feature = "ffi", ffier::export)]
pub fn double_foreign_item_score(
    #[cfg_attr(feature = "ffi", ffier(foreign = ::ffier_test_foreign_lib_via_cdylib, c_name = "FlForeignItem"))]
    item: &mut ::ffier_test_foreign_lib_via_cdylib::ForeignItem,
) {
    let current = item.score();
    item.set_score(current * 2);
}

/// Consume a foreign item and return its score (tests by-value foreign param).
#[cfg_attr(feature = "ffi", ffier::export)]
pub fn consume_foreign_item(
    #[cfg_attr(feature = "ffi", ffier(foreign = ::ffier_test_foreign_lib_via_cdylib, c_name = "FlForeignItem"))]
    item: ffier_test_foreign_lib_via_cdylib::ForeignItem,
) -> i32 {
    item.score()
}

/// Create a foreign config, returning Result with foreign ok type.
#[cfg_attr(feature = "ffi", ffier::export)]
#[cfg_attr(feature = "ffi", ffier(foreign_return = ::ffier_test_foreign_lib_via_cdylib, c_name = "FlForeignConfig"))]
pub fn create_foreign_config_checked(
    name: &str,
    value: i32,
) -> Result<ffier_test_foreign_lib_via_cdylib::ForeignConfig, TestError> {
    if name.is_empty() {
        Err(TestError::NotFound("empty name".into()))
    } else {
        Ok(ffier_test_foreign_lib_via_cdylib::ForeignConfig::new(
            name, value,
        ))
    }
}

// ---------------------------------------------------------------------------
// Opaque raw pointer tests — *mut c_void passed through without transformation
// ---------------------------------------------------------------------------

use core::ffi::c_void;

/// Accept an opaque pointer and return it unchanged (round-trip test).
/// Uses bare `c_void` (via `use`) to verify the macro emits fully qualified paths.
#[cfg_attr(feature = "ffi", ffier::export)]
pub fn opaque_round_trip(ptr: *mut c_void) -> *mut c_void {
    ptr
}

/// Accept an opaque const pointer and return its address as an integer.
#[cfg_attr(feature = "ffi", ffier::export)]
pub fn opaque_ptr_to_int(ptr: *const c_void) -> usize {
    ptr as usize
}

#[cfg(test)]
mod error_tests {
    use super::TestError;
    use anyhow::anyhow;
    #[cfg(feature = "ffi")]
    use ffier::FfiError as _;

    #[test]
    fn fatal_variant_display_forwards_to_inner_anyhow() {
        let err = TestError::Fatal(anyhow!("disk is full"));
        assert_eq!(err.to_string(), "disk is full");
    }

    #[test]
    #[cfg(feature = "ffi")]
    fn fatal_variant_ffi_code() {
        let err = TestError::Fatal(anyhow!("boom"));
        assert_eq!(err.code(), 5);
    }

    #[test]
    fn fatal_variant_anyhow_chain_is_preserved() {
        use anyhow::Context as _;
        let root: anyhow::Result<()> = Err(anyhow!("connection refused"));
        let chained = root.context("failed to open socket");
        let err = TestError::Fatal(chained.unwrap_err());
        // Display shows only the outermost context message
        assert_eq!(err.to_string(), "failed to open socket");
    }
}

#[cfg(test)]
mod fieldless_error_client_tests {
    use super as api;

    fn init() {}

    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fieldless_error_client.rs"
    ));
}

// ---------------------------------------------------------------------------
// Library metadata — lists all exported types for batched generation
// ---------------------------------------------------------------------------

#[cfg(feature = "ffi")]
ffier::library_definition!("ft", library_tag = 1,
    TestError = 1,
    trait ErrorForwarder = 30,
    fn test_error_drop_count,
    fn roundtrip_callback_error,
    Widget = 2, Gadget = 3, Config = 4,
    Gizmo = 5, GizmoBuilder = 6,
    View<'_> = 7, ViewFactory = 8,
    Pipeline = 9,
    trait Processor = 10,
    trait EmptyTrait = 31,
    trait RetiredTrait = 32,
    Apple = 11, Orange = 12, Banana = 13, Mango = 14,
    Peach = 15, Plum = 16, Grape = 17, Lemon = 18,
    trait Fruit = 20,
    Fruit for Apple,
    Fruit for Orange,
    Fruit for Banana,
    Fruit for Mango,
    Fruit for Peach,
    Fruit for Plum,
    Fruit for Grape,
    Fruit for Lemon,
    Mixer = 21,
    Sprocket = 22,
    Attachment for Sprocket,
    Snapshot for View,
    Snapshot for Widget,
    Snapshot for Gadget,
    trait Weighable = 23,
    Weighable for Apple,
    trait Categorizable = 29,
    Categorizable for Apple,
    Categorizable for Orange,
    trait ffier_builtins::PushStr = 24,
    trait ffier_builtins::Error = 25,
    Error for TestError,
    #[cfg(feature = "optional-entry")]
    crate::optional_api::OptionalWidget = 27,
    #[cfg(feature = "optional-entry")]
    trait crate::optional_api::OptionalWorker = 28,
    #[cfg(feature = "optional-entry")]
    crate::optional_api::OptionalWorker for crate::optional_api::OptionalWidget,
    enum LogLevel,
    #[cfg(feature = "optional-entry")]
    enum crate::optional_api::OptionalMode,
    #[cfg(feature = "optional-entry")]
    bitflags crate::optional_api::OptionalFlags,
    #[cfg(feature = "optional-entry")]
    value crate::optional_api::OptionalSample,
    bitflags Permissions,
    value EventPayload,
    value InputEvent,
    trait SampleSource = 33,
    FixedSource = 34,
    SampleSource for FixedSource,
    fn log_level_name,
    fn log_level_is_enabled,
    #[cfg(feature = "optional-entry")]
    fn crate::optional_api::optional_apply,
    #[cfg(feature = "optional-entry")]
    fn crate::optional_api::optional_mode_name,
    #[cfg(feature = "optional-entry")]
    fn crate::optional_api::optional_merge_flags,
    #[cfg(feature = "optional-entry")]
    fn crate::optional_api::optional_sample,
    fn categorizable_value,
    fn clone_fd,
    fn sum_gadget_values,
    fn event_sequence,
    fn event_payload_code,
    fn event_bump_payload_code,
    fn event_make,
    fn event_checked,
    fn source_next_sequence,
    fn sample_at,
    fn opaque_round_trip,
    fn opaque_ptr_to_int,
    fn apply_foreign_config,
    fn read_foreign_item_score,
    fn double_foreign_item_score,
    fn consume_foreign_item,
    fn create_foreign_item,
    fn create_foreign_config_checked,
);

#[cfg(feature = "ffi")]
ffier::generate_bridge!(
    local = __ffier_ft_metadata,
    schema_output = "../../target/ffier-ft.json"
);

// ---------------------------------------------------------------------------
// Manual bridge function — peeks at a handle's type tag to verify dispatch path.
// Also demonstrates that hand-written bridge functions work alongside generated ones.
// ---------------------------------------------------------------------------

/// Returns the concrete type name inside the handle ("Apple", "Orange",
/// "VtableFruit", or "unknown"). Consumes and destroys the handle.
///
/// # Safety
/// `handle` must be a valid handle to an `Apple`, `Orange`, or `VtableFruit`,
/// or `NULL`. The handle is consumed and must not be used after this call.
#[cfg(feature = "ffi")]
#[unsafe(no_mangle)]
#[allow(clippy::drop_non_drop)]
pub unsafe extern "C" fn ft_debug_fruit_dispatch_kind(
    handle: *mut core::ffi::c_void,
) -> ffier::FfierBytes {
    unsafe {
        use FfiHandle;
        use FfiType;
        let tag = ffier::handle_type_tag(handle);
        let name = if tag == VtableFruit::TYPE_TAG {
            drop(<VtableFruit as FfiType>::from_c(handle));
            "VtableFruit"
        } else if tag == Apple::TYPE_TAG {
            drop(<Apple as FfiType>::from_c(handle));
            "Apple"
        } else if tag == Orange::TYPE_TAG {
            drop(<Orange as FfiType>::from_c(handle));
            "Orange"
        } else {
            "unknown"
        };
        // SAFETY: returned FfierBytes points to a static string literal — outlives the call.
        ffier::FfierBytes::from_str(name)
    }
}

#[cfg(all(test, feature = "ffi"))]
mod tests {
    use super::*;
    use FfiHandle;
    use std::ffi::{CStr, c_void};
    use std::ptr;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// Helper: stream ft_error_message into a String via a temporary PushStr handle.
    unsafe fn error_message_to_string(err: *mut core::ffi::c_void) -> String {
        use core::ffi::c_void;

        // Vtable push callback: appends to a String via user_data.
        unsafe extern "C" fn push_to_string(
            self_data: *mut c_void,
            data: ffier::FfierBytes,
        ) -> bool {
            let s = unsafe { &mut *(self_data as *mut String) };
            s.push_str(unsafe { data.as_str_unchecked() });
            true
        }

        let vtable = PushStrVtable {
            _num_slots: 1,
            _reserved1: 0,
            _reserved2: 0,
            drop: None,
            push: Some(push_to_string),
        };

        let mut buf = String::new();
        let mut handle = ffier::FfierHandle {
            type_tag: VtablePushStr::TYPE_TAG,
            metadata: 0,
            value: ffier::VtableHandle {
                vtable_ptr: &vtable as *const _ as *const c_void,
                user_data: &mut buf as *mut String as *const c_void,
            },
        };

        unsafe {
            ft_error_message(err as *mut c_void, &mut handle as *mut _ as *mut c_void);
        }
        buf
    }

    // ================================================================
    // Constructors
    // ================================================================

    #[test]
    fn static_method_returning_self() {
        unsafe {
            let w = ft_widget_new();
            assert_eq!(ft_widget_get_count(w), 0);
            ft_widget_destroy(w);
        }
    }

    #[test]
    fn static_method_returning_self_with_str_param() {
        unsafe {
            let w = ft_widget_with_name(ffier::FfierBytes::from_str("hello"));
            assert_eq!(ft_widget_name(w).as_str_unchecked(), "hello");
            ft_widget_destroy(w);
        }
    }

    // ================================================================
    // Receiver patterns
    // ================================================================

    #[test]
    fn immutable_ref_method_returning_primitive() {
        unsafe {
            let w = ft_widget_new();
            assert_eq!(ft_widget_get_count(w), 0);
            ft_widget_destroy(w);
        }
    }

    #[test]
    fn mutable_ref_method_void_return() {
        unsafe {
            let w = ft_widget_new();
            ft_widget_set_count(w, 42);
            assert_eq!(ft_widget_get_count(w), 42);
            ft_widget_destroy(w);
        }
    }

    #[test]
    fn by_value_method_void_return() {
        unsafe {
            let w = ft_widget_new();
            ft_widget_consume(w);
        }
    }

    // ================================================================
    // Primitive param/return types
    // ================================================================

    #[test]
    fn method_returning_bool() {
        unsafe {
            let w = ft_widget_new();
            assert!(ft_widget_is_active(w));
            ft_widget_destroy(w);
        }
    }

    #[test]
    fn method_with_i64_param_returning_i64() {
        unsafe {
            let w = ft_widget_new();
            assert_eq!(ft_widget_negate(w, 42), -42);
            assert_eq!(ft_widget_negate(w, -100), 100);
            assert_eq!(ft_widget_negate(w, 0), 0);
            ft_widget_destroy(w);
        }
    }

    // ================================================================
    // String/bytes returns
    // ================================================================

    #[test]
    fn method_returning_str() {
        unsafe {
            let w = ft_widget_new();
            assert_eq!(ft_widget_name(w).as_str_unchecked(), "widget");
            ft_widget_destroy(w);
        }
    }

    #[test]
    fn method_returning_bytes() {
        unsafe {
            let w = ft_widget_with_name(ffier::FfierBytes::from_str("abc"));
            assert_eq!(ft_widget_data(w).as_bytes(), b"abc");
            ft_widget_destroy(w);
        }
    }

    #[test]
    fn method_with_str_param_returning_str() {
        unsafe {
            let w = ft_widget_new();
            assert_eq!(
                ft_widget_echo(w, ffier::FfierBytes::from_str("ping")).as_str_unchecked(),
                "ping"
            );
            ft_widget_destroy(w);
        }
    }

    // ================================================================
    // Str slice param
    // ================================================================

    #[test]
    fn method_with_str_slice_param() {
        unsafe {
            let w = ft_widget_new();
            let tags = [
                ffier::FfierBytes::from_str("alpha"),
                ffier::FfierBytes::from_str("beta"),
                ffier::FfierBytes::from_str("gamma"),
            ];
            ft_widget_set_tags(w, tags.as_ptr(), 3);
            assert_eq!(
                ft_widget_tags_joined(w).as_str_unchecked(),
                "alpha,beta,gamma"
            );
            ft_widget_destroy(w);
        }
    }

    // ================================================================
    // File descriptors — skipped under Miri (requires real syscalls)
    // ================================================================

    #[test]
    #[cfg_attr(miri, ignore)]
    fn method_with_borrowed_fd_param() {
        unsafe {
            let w = ft_widget_new();
            assert_eq!(ft_widget_fd_number(w, 0), 0); // stdin
            ft_widget_destroy(w);
        }
    }

    #[test]
    #[cfg_attr(miri, ignore)]
    fn method_with_borrowed_fd_returning_owned_fd() {
        unsafe {
            use std::os::unix::io::FromRawFd;
            let w = ft_widget_new();
            let new_fd = ft_widget_dup_fd(w, 1); // dup stdout
            assert!(new_fd >= 0);
            assert_ne!(new_fd, 1);
            drop(std::os::unix::io::OwnedFd::from_raw_fd(new_fd));
            ft_widget_destroy(w);
        }
    }

    #[test]
    #[cfg_attr(miri, ignore)]
    fn method_with_optional_owned_fd_return_some() {
        unsafe {
            use std::os::unix::io::FromRawFd;
            let w = ft_widget_new();
            let mut result: i32 = -1;
            let mut err_out: *mut core::ffi::c_void = core::ptr::null_mut();
            let r = ft_widget_maybe_owned_fd(w, 1, &mut result, &mut err_out);
            assert_eq!(r, 0, "should succeed");
            assert!(result >= 0, "should return a valid fd");
            drop(std::os::unix::io::OwnedFd::from_raw_fd(result));
            ft_widget_destroy(w);
        }
    }

    #[test]
    #[cfg_attr(miri, ignore)]
    fn method_with_optional_owned_fd_return_none() {
        unsafe {
            let w = ft_widget_new();
            let mut result: i32 = -1;
            let mut err_out: *mut core::ffi::c_void = core::ptr::null_mut();
            let r = ft_widget_maybe_owned_fd(w, 0, &mut result, &mut err_out);
            assert_eq!(r, 0, "should succeed");
            assert_eq!(result, -1, "None should map to -1");
            ft_widget_destroy(w);
        }
    }

    #[test]
    #[cfg_attr(miri, ignore)]
    fn method_with_optional_borrowed_fd_param_dup_some() {
        unsafe {
            use std::os::unix::io::FromRawFd;
            let w = ft_widget_new();
            let mut result: i32 = -1;
            let mut err_out: *mut core::ffi::c_void = core::ptr::null_mut();
            // Pass stdout (fd 1) as Some
            let r = ft_widget_maybe_dup_fd(w, 1, &mut result, &mut err_out);
            assert_eq!(r, 0, "should succeed");
            assert!(result >= 0, "should return a valid fd");
            assert_ne!(result, 1, "should be a new fd, not the original");
            drop(std::os::unix::io::OwnedFd::from_raw_fd(result));
            ft_widget_destroy(w);
        }
    }

    #[test]
    #[cfg_attr(miri, ignore)]
    fn method_with_optional_borrowed_fd_param_dup_none() {
        unsafe {
            let w = ft_widget_new();
            let mut result: i32 = -1;
            let mut err_out: *mut core::ffi::c_void = core::ptr::null_mut();
            // Pass -1 as None
            let r = ft_widget_maybe_dup_fd(w, -1, &mut result, &mut err_out);
            assert_eq!(r, 0, "should succeed");
            assert_eq!(result, -1, "None input should produce None output (-1)");
            ft_widget_destroy(w);
        }
    }

    // ================================================================
    // Result return patterns
    // ================================================================

    #[test]
    fn method_returning_result_void_ok() {
        unsafe {
            let w = ft_widget_new();
            let mut err: *mut core::ffi::c_void = ptr::null_mut();
            let r = ft_widget_validate(w, &mut err as *mut *mut core::ffi::c_void);
            assert_eq!(r, 0);
            ft_widget_destroy(w);
        }
    }

    #[test]
    fn method_returning_result_void_err() {
        unsafe {
            let w = ft_widget_new();
            let mut err: *mut core::ffi::c_void = ptr::null_mut();
            let r = ft_widget_fail_always(w, &mut err as *mut *mut core::ffi::c_void);
            assert_ne!(r, 0);
            assert_eq!(ffier::ffier_result_code(r), 2); // CustomMessage
            ft_error_destroy(err);
            ft_widget_destroy(w);
        }
    }

    #[test]
    fn method_returning_result_value_ok() {
        unsafe {
            let w = ft_widget_new();
            let mut result: i32 = -1;
            let mut err: *mut core::ffi::c_void = ptr::null_mut();
            let r = ft_widget_parse_count(
                w,
                ffier::FfierBytes::from_str("hello"),
                &mut result,
                &mut err as *mut *mut core::ffi::c_void,
            );
            assert_eq!(r, 0);
            assert_eq!(result, 5);
            ft_widget_destroy(w);
        }
    }

    #[test]
    fn method_returning_result_value_err() {
        unsafe {
            let w = ft_widget_new();
            let mut result: i32 = -1;
            let mut err: *mut core::ffi::c_void = ptr::null_mut();
            let r = ft_widget_parse_count(
                w,
                ffier::FfierBytes::from_str("error"),
                &mut result,
                &mut err as *mut *mut core::ffi::c_void,
            );
            assert_ne!(r, 0);
            assert_eq!(ffier::ffier_result_code(r), 1); // NotFound
            ft_error_destroy(err);
            ft_widget_destroy(w);
        }
    }

    #[test]
    fn method_returning_result_str_ok() {
        unsafe {
            let w = ft_widget_new();
            let mut result = ffier::FfierBytes::EMPTY;
            let mut err: *mut core::ffi::c_void = ptr::null_mut();
            let r = ft_widget_describe(w, 0, &mut result, &mut err as *mut *mut core::ffi::c_void);
            assert_eq!(r, 0);
            assert_eq!(result.as_str_unchecked(), "zero");
            ft_widget_destroy(w);
        }
    }

    #[test]
    fn method_returning_result_str_err() {
        unsafe {
            let w = ft_widget_new();
            let mut result = ffier::FfierBytes::EMPTY;
            let mut err: *mut core::ffi::c_void = ptr::null_mut();
            let r = ft_widget_describe(w, 99, &mut result, &mut err as *mut *mut core::ffi::c_void);
            assert_ne!(r, 0);
            assert_eq!(ffier::ffier_result_code(r), 1); // NotFound
            assert!(!err.is_null(), "err handle should be non-null");
            // Test error_payload borrows the Box<str> payload from the handle
            let mut payload_buf = core::mem::MaybeUninit::<ffier::FfierBytes>::uninit();
            ft_error_payload(
                err as *const core::ffi::c_void,
                0,
                payload_buf.as_mut_ptr() as *mut core::ffi::c_void,
                core::mem::size_of::<ffier::FfierBytes>(),
            );
            let c_val = payload_buf.assume_init();
            let s =
                core::str::from_utf8_unchecked(core::slice::from_raw_parts(c_val.data, c_val.len));
            assert_eq!(s, "code 99");
            // No need to free — c_val borrows from the handle
            ft_error_destroy(err);
            ft_widget_destroy(w);
        }
    }

    #[test]
    fn error_payload_i32() {
        unsafe {
            let w = ft_widget_new();
            let mut err: *mut core::ffi::c_void = ptr::null_mut();
            let r = ft_widget_fail_with_number(w, 42, &mut err as *mut *mut core::ffi::c_void);
            assert_ne!(r, 0);
            assert_eq!(ffier::ffier_result_code(r), 4); // NumericError
            assert!(!err.is_null());
            // Test error_payload with i32-sized buffer
            let mut payload_buf = core::mem::MaybeUninit::<i32>::uninit();
            ft_error_payload(
                err as *const core::ffi::c_void,
                0,
                payload_buf.as_mut_ptr() as *mut core::ffi::c_void,
                core::mem::size_of::<i32>(),
            );
            assert_eq!(payload_buf.assume_init(), 42);
            ft_error_destroy(err);
            ft_widget_destroy(w);
        }
    }

    #[test]
    fn error_payload_indices_preserve_opaque_slots() {
        unsafe {
            let error = TestError::MixedError(17, anyhow::anyhow!("hidden detail"), 900);
            let handle = error.into_c();
            let mut first = 0i32;
            let mut last = 0u64;
            ft_error_payload(
                handle,
                0,
                &mut first as *mut i32 as *mut c_void,
                size_of::<i32>(),
            );
            ft_error_payload(
                handle,
                2,
                &mut last as *mut u64 as *mut c_void,
                size_of::<u64>(),
            );
            assert_eq!((first, last), (17, 900));
            ft_error_destroy(handle);
        }
    }

    #[test]
    #[should_panic(expected = "TestError::MixedError field 1 is opaque")]
    fn error_payload_rejects_opaque_index() {
        let error = TestError::MixedError(17, anyhow::anyhow!("hidden detail"), 900);
        let mut output = 0u64;
        unsafe {
            ffier::FfiError::payload(
                &error,
                1,
                (&mut output as *mut u64).cast(),
                size_of::<u64>(),
            );
        }
    }

    #[test]
    #[should_panic(expected = "TestError::MixedError field index 3 out of bounds for 3 fields")]
    fn error_payload_rejects_out_of_bounds_index() {
        let error = TestError::MixedError(17, anyhow::anyhow!("hidden detail"), 900);
        let mut output = 0u64;
        unsafe {
            ffier::FfiError::payload(
                &error,
                3,
                (&mut output as *mut u64).cast(),
                size_of::<u64>(),
            );
        }
    }

    #[test]
    #[should_panic(expected = "out of bounds for 3 fields")]
    fn error_payload_rejects_maximum_index() {
        let error = TestError::MixedError(17, anyhow::anyhow!("hidden detail"), 900);
        let mut output = 0u64;
        unsafe {
            ffier::FfiError::payload(
                &error,
                usize::MAX,
                (&mut output as *mut u64).cast(),
                size_of::<u64>(),
            );
        }
    }

    #[test]
    #[should_panic(expected = "TestError::CustomMessage field index 0 out of bounds for 0 fields")]
    fn error_payload_rejects_unit_variant_access() {
        let error = TestError::CustomMessage;
        let mut output = 0u64;
        unsafe {
            ffier::FfiError::payload(
                &error,
                0,
                (&mut output as *mut u64).cast(),
                size_of::<u64>(),
            );
        }
    }

    #[test]
    #[should_panic(expected = "TestError::InvalidInput field index 0 out of bounds for 0 fields")]
    fn error_payload_rejects_empty_tuple_access() {
        let error = TestError::InvalidInput();
        let mut output = 0u64;
        unsafe {
            ffier::FfiError::payload(
                &error,
                0,
                (&mut output as *mut u64).cast(),
                size_of::<u64>(),
            );
        }
    }

    #[test]
    #[should_panic(expected = "TestError::OpaqueUnit field 0 is opaque")]
    fn error_payload_rejects_wholly_opaque_variant_access() {
        let error = TestError::OpaqueUnit(());
        let mut output = 0u64;
        unsafe {
            ffier::FfiError::payload(
                &error,
                0,
                (&mut output as *mut u64).cast(),
                size_of::<u64>(),
            );
        }
    }

    #[test]
    #[should_panic(expected = "TestError::Fatal field 0 is opaque")]
    fn error_payload_rejects_variant_level_opaque_access() {
        let error = TestError::Fatal(anyhow::anyhow!("hidden detail"));
        let mut output = 0u64;
        unsafe {
            ffier::FfiError::payload(
                &error,
                0,
                (&mut output as *mut u64).cast(),
                size_of::<u64>(),
            );
        }
    }

    #[test]
    #[should_panic(expected = "error payload buffer too small")]
    fn error_payload_rejects_small_buffer() {
        let error = TestError::NumericError(17);
        let mut output = 0u64;
        unsafe {
            ffier::FfiError::payload(&error, 0, (&mut output as *mut u64).cast(), 0);
        }
    }

    #[test]
    fn opaque_unit_payload_is_not_an_empty_tuple() {
        use ffier::FfiError as _;

        assert!(TestError::from_code(7).is_none());
        assert!(ft_error_new(ffier::ffier_result(TestError::TYPE_TAG, 7)).is_null());
        let error = TestError::OpaqueUnit(());
        assert!(matches!(error, TestError::OpaqueUnit(())));
    }

    #[test]
    fn method_returning_result_handle_ok() {
        unsafe {
            let w = ft_widget_new();
            ft_widget_set_count(w, 7);
            let mut err: *mut core::ffi::c_void = ptr::null_mut();
            let g = ft_widget_try_create_gadget(w, true, &mut err as *mut *mut core::ffi::c_void);
            assert!(!g.is_null());
            assert_eq!(ft_gadget_get(g), 7);
            ft_gadget_destroy(g);
            ft_widget_destroy(w);
        }
    }

    #[test]
    fn method_returning_result_handle_err() {
        unsafe {
            let w = ft_widget_new();
            let mut err: *mut core::ffi::c_void = ptr::null_mut();
            let g = ft_widget_try_create_gadget(w, false, &mut err as *mut *mut core::ffi::c_void);
            assert!(g.is_null());
            // Extract result code from error handle
            let r2 = ft_error_result(err as *mut core::ffi::c_void);
            assert_eq!(ffier::ffier_result_code(r2), 1); // NotFound
            ft_error_destroy(err);
            ft_widget_destroy(w);
        }
    }

    #[test]
    fn result_name_data_carrying() {
        unsafe {
            let w = ft_widget_new();
            let mut result: i32 = -1;
            let mut err: *mut core::ffi::c_void = ptr::null_mut();
            let r = ft_widget_parse_count(
                w,
                ffier::FfierBytes::from_str("error"),
                &mut result,
                &mut err as *mut *mut core::ffi::c_void,
            );
            assert_ne!(r, 0);
            // Names never include payload hints or message overrides.
            let msg = CStr::from_ptr(ft_result_name_cstr(r)).to_str().unwrap();
            assert_eq!(msg, "NotFound");
            ft_error_destroy(err);
            ft_widget_destroy(w);
        }
    }

    #[test]
    fn error_message_has_interpolated_data() {
        unsafe {
            let w = ft_widget_new();
            let mut result: i32 = -1;
            let mut err: *mut core::ffi::c_void = ptr::null_mut();
            let r = ft_widget_parse_count(
                w,
                ffier::FfierBytes::from_str("error"),
                &mut result,
                &mut err as *mut *mut core::ffi::c_void,
            );
            assert_ne!(r, 0);
            // error_message streams the rich Display output with interpolated data
            let msg = error_message_to_string(err);
            assert_eq!(msg, "not found: error");
            // Static lookup cannot interpolate the payload.
            let static_msg = CStr::from_ptr(ft_result_name_cstr(r)).to_str().unwrap();
            assert_eq!(static_msg, "NotFound");
            assert_eq!(ft_result_message(r).as_str_unchecked(), "NotFound");
            ft_error_destroy(err);
            ft_widget_destroy(w);
        }
    }

    // ================================================================
    // Handle as parameter
    // ================================================================

    #[test]
    fn method_with_borrowed_handle_param() {
        unsafe {
            let w = ft_widget_new();
            ft_widget_set_count(w, 10);
            let g = ft_widget_create_gadget(w);
            assert_eq!(ft_widget_read_gadget(w, g), 10);
            ft_gadget_destroy(g);
            ft_widget_destroy(w);
        }
    }

    #[test]
    fn method_with_mutable_handle_param() {
        unsafe {
            let w = ft_widget_new();
            let g = ft_widget_create_gadget(w);
            assert_eq!(ft_gadget_get(g), 0);
            ft_widget_update_gadget(w, g, 99);
            assert_eq!(ft_gadget_get(g), 99);
            ft_gadget_destroy(g);
            ft_widget_destroy(w);
        }
    }

    #[test]
    fn method_returning_handle() {
        unsafe {
            let w = ft_widget_new();
            ft_widget_set_count(w, 33);
            let g = ft_widget_create_gadget(w);
            assert_eq!(ft_gadget_get(g), 33);
            ft_gadget_destroy(g);
            ft_widget_destroy(w);
        }
    }

    #[test]
    fn method_returning_borrowed_handle() {
        unsafe {
            let w = ft_widget_new();
            // The widget has an internal gadget with value 42.
            let g = ft_widget_gadget(w);
            assert!(!g.is_null());

            // The returned handle has a valid type tag — we can call methods on it.
            assert_eq!(ft_gadget_get(g), 42);

            // Destroying a borrowed handle is safe (deallocates the shell,
            // does NOT drop the inner Gadget which still lives in Widget).
            ft_gadget_destroy(g);

            // The widget is still fully alive after destroying the borrowed handle.
            ft_widget_set_count(w, 7);
            assert_eq!(ft_widget_get_count(w), 7);

            ft_widget_destroy(w);
        }
    }

    // ================================================================
    // Builder pattern (by-value self -> Self)
    // ================================================================

    #[test]
    fn builder_method_returning_self() {
        unsafe {
            let mut c = ft_config_new();
            ft_config_set_name(
                &mut c as *mut *mut core::ffi::c_void as *mut core::ffi::c_void,
                ffier::FfierBytes::from_str("myconfig"),
            );
            ft_config_set_size(
                &mut c as *mut *mut core::ffi::c_void as *mut core::ffi::c_void,
                42,
            );
            assert_eq!(ft_config_get_name(c).as_str_unchecked(), "myconfig");
            assert_eq!(ft_config_get_size(c), 42);
            ft_config_destroy(c);
        }
    }

    #[test]
    fn builder_method_returning_result_self_ok() {
        unsafe {
            let mut c = ft_config_new();
            ft_config_set_name(
                &mut c as *mut *mut core::ffi::c_void as *mut core::ffi::c_void,
                ffier::FfierBytes::from_str("valid"),
            );
            let mut err: *mut core::ffi::c_void = ptr::null_mut();
            let r = ft_config_validated(
                &mut c as *mut *mut core::ffi::c_void as *mut core::ffi::c_void,
                &mut err as *mut *mut core::ffi::c_void,
            );
            assert_eq!(r, 0);
            assert_eq!(ft_config_get_name(c).as_str_unchecked(), "valid");
            ft_config_destroy(c);
        }
    }

    #[test]
    fn builder_method_returning_result_self_err() {
        unsafe {
            let mut c = ft_config_new();
            // name is empty — validated() should fail
            let mut err: *mut core::ffi::c_void = ptr::null_mut();
            let r = ft_config_validated(
                &mut c as *mut *mut core::ffi::c_void as *mut core::ffi::c_void,
                &mut err as *mut *mut core::ffi::c_void,
            );
            assert_ne!(r, 0);
            assert_eq!(ffier::ffier_result_code(r), 3); // InvalidInput
            // After error with by-value self, handle is consumed
            assert!(!err.is_null());
            ft_error_destroy(err);
        }
    }

    #[test]
    fn builder_consuming_self_returning_other_handle() {
        unsafe {
            let b = ft_gizmo_builder_new();
            ft_gizmo_builder_set_name(b, ffier::FfierBytes::from_str("mygizmo"));
            ft_gizmo_builder_set_size(b, 100);
            let g = ft_gizmo_builder_build(b);
            // b is consumed
            assert_eq!(ft_gizmo_name(g).as_str_unchecked(), "mygizmo");
            assert_eq!(ft_gizmo_size(g), 100);
            ft_gizmo_destroy(g);
        }
    }

    #[test]
    fn builder_consuming_self_returning_result_handle_ok() {
        unsafe {
            let b = ft_gizmo_builder_new();
            ft_gizmo_builder_set_name(b, ffier::FfierBytes::from_str("valid"));
            ft_gizmo_builder_set_size(b, 50);
            let mut err: *mut core::ffi::c_void = ptr::null_mut();
            let g = ft_gizmo_builder_try_build(b, &mut err as *mut *mut core::ffi::c_void);
            // b is consumed
            assert!(!g.is_null());
            assert_eq!(ft_gizmo_name(g).as_str_unchecked(), "valid");
            assert_eq!(ft_gizmo_size(g), 50);
            ft_gizmo_destroy(g);
        }
    }

    #[test]
    fn builder_consuming_self_returning_result_handle_err() {
        unsafe {
            let b = ft_gizmo_builder_new();
            // name empty — try_build() should fail
            let mut err: *mut core::ffi::c_void = ptr::null_mut();
            let g = ft_gizmo_builder_try_build(b, &mut err as *mut *mut core::ffi::c_void);
            // b is consumed
            assert!(g.is_null());
            let r2 = ft_error_result(err as *mut core::ffi::c_void);
            assert_eq!(ffier::ffier_result_code(r2), 3); // InvalidInput
            ft_error_destroy(err);
        }
    }

    // ================================================================
    // Error type FFI
    // ================================================================

    #[test]
    fn error_code_constants() {
        use ffier::FfiError;
        let codes = TestError::codes();
        assert!(
            codes
                .iter()
                .any(|&(name, val)| name == "NOT_FOUND" && val == 1)
        );
        assert!(
            codes
                .iter()
                .any(|&(name, val)| name == "CUSTOM_MESSAGE" && val == 2)
        );
        assert!(
            codes
                .iter()
                .any(|&(name, val)| name == "INVALID_INPUT" && val == 3)
        );
    }

    #[test]
    fn result_name_is_independent_of_explicit_static_message() {
        let before = test_error_drop_count();
        let result = ffier::ffier_result(TestError::TYPE_TAG, 2);
        unsafe {
            let msg = CStr::from_ptr(ft_result_name_cstr(result))
                .to_str()
                .unwrap();
            assert_eq!(msg, "CustomMessage");
            assert_eq!(ft_result_name(result).as_str_unchecked(), "CustomMessage");
            assert_eq!(
                ft_result_message(result).as_str_unchecked(),
                "custom error message"
            );
        }
        assert_eq!(test_error_drop_count(), before);
    }

    #[test]
    fn result_name_success() {
        unsafe {
            let msg = ft_result_name(0);
            assert_eq!(msg.as_str_unchecked(), "success");
            assert_eq!(ft_result_message(0).as_str_unchecked(), "success");
        }
    }

    #[test]
    fn result_lookup_unknown_values_need_no_error_handle() {
        for (result, message) in [
            (ffier::ffier_result(u32::MAX, 1), "unknown error"),
            (
                ffier::ffier_result(TestError::TYPE_TAG, u32::MAX),
                "unknown test error error",
            ),
        ] {
            unsafe {
                assert_eq!(ft_result_name(result).as_str_unchecked(), "unknown error");
                assert_eq!(ft_result_message(result).as_str_unchecked(), message);
            }
        }
    }

    #[test]
    fn result_type_tag_and_code() {
        unsafe {
            let w = ft_widget_new();
            let mut err: *mut core::ffi::c_void = ptr::null_mut();
            let r = ft_widget_fail_always(w, &mut err as *mut *mut core::ffi::c_void);
            // TestError has type_tag=1, library_tag=1 → composed tag
            let expected_tag = <TestError as FfiHandle>::TYPE_TAG;
            assert_eq!(ffier::ffier_result_type_tag(r), expected_tag);
            assert_eq!(ffier::ffier_result_code(r), 2);
            ft_error_destroy(err);
            ft_widget_destroy(w);
        }
    }

    #[test]
    fn error_handle_message_and_destroy() {
        unsafe {
            let w = ft_widget_new();
            let mut err: *mut core::ffi::c_void = ptr::null_mut();
            let r = ft_widget_fail_always(w, &mut err as *mut *mut core::ffi::c_void);
            assert_ne!(r, 0);
            // ft_error_message streams the Display output through PushStr
            let msg = error_message_to_string(err);
            assert_eq!(msg, "custom error message");
            ft_error_destroy(err);
            ft_widget_destroy(w);
        }
    }

    #[test]
    fn error_factory_constructs_only_native_fieldless_variants() {
        use ffier::FfiError as _;

        for code in [0, 1, 4, 5, 6, 7, 8, u32::MAX] {
            assert!(TestError::from_code(code).is_none());
            assert!(ft_error_new(ffier::ffier_result(TestError::TYPE_TAG, code)).is_null());
        }
        assert!(ft_error_new(0).is_null());
        assert!(ft_error_new(ffier::ffier_result(u32::MAX, 2)).is_null());
        for (code, message) in [(2, "custom error message"), (3, "invalid input")] {
            let before = test_error_drop_count();
            let handle = ft_error_new(ffier::ffier_result(TestError::TYPE_TAG, code));
            assert!(!handle.is_null());
            unsafe {
                assert_eq!(ffier::handle_type_tag(handle), TestError::TYPE_TAG);
                assert_eq!(error_message_to_string(handle), message);
                let error = TestError::from_c(handle);
                assert_eq!(ffier::FfiError::code(&error), code);
                match code {
                    2 => assert!(matches!(error, TestError::CustomMessage)),
                    3 => assert!(matches!(error, TestError::InvalidInput())),
                    _ => unreachable!(),
                }
                assert_eq!(test_error_drop_count(), before);
                drop(error);
            }
            assert_eq!(test_error_drop_count(), before + 1);
        }
    }

    #[test]
    fn error_result_from_handle() {
        unsafe {
            let w = ft_widget_new();
            let mut err: *mut core::ffi::c_void = ptr::null_mut();
            let r = ft_widget_fail_always(w, &mut err as *mut *mut core::ffi::c_void);
            assert_ne!(r, 0);
            // ft_error_result extracts the FtResult from the boxed error
            let r2 = ft_error_result(err as *mut core::ffi::c_void);
            assert_eq!(r, r2);
            ft_error_destroy(err);
            ft_widget_destroy(w);
        }
    }

    #[test]
    fn error_result_null_returns_success() {
        // ft_error_result is now a trait dispatch method — no null guard.
    }

    #[test]
    fn error_handle_has_rtti_type_tag() {
        unsafe {
            let w = ft_widget_new();
            let mut err: *mut core::ffi::c_void = ptr::null_mut();
            let r = ft_widget_fail_always(w, &mut err as *mut *mut core::ffi::c_void);
            assert_ne!(r, 0);
            // The error handle is a proper FfierHandleBox — type tag is readable
            let tag = ffier::handle_type_tag(err as *const core::ffi::c_void);
            // TestError has type_tag=1, library_tag=1 → composed tag
            let expected_tag = <TestError as FfiHandle>::TYPE_TAG;
            assert_eq!(tag, expected_tag);
            ft_error_destroy(err);
            ft_widget_destroy(w);
        }
    }

    #[test]
    fn error_handle_null_is_safe() {
        unsafe {
            // Passing NULL err_out is fine — no box is written
            let w = ft_widget_new();
            let mut err: *mut core::ffi::c_void = ptr::null_mut();
            let r = ft_widget_fail_always(w, &mut err as *mut *mut core::ffi::c_void);
            assert_ne!(r, 0);
            ft_error_destroy(err);
            ft_widget_destroy(w);

            // Destroying NULL is a no-op
            ft_error_destroy(ptr::null_mut());

            // ft_error_result on NULL returns SUCCESS
            // ft_error_result is now a trait dispatch method — no null guard.
        }
    }

    #[test]
    fn error_handle_not_written_on_success() {
        unsafe {
            let w = ft_widget_new();
            let mut err: *mut core::ffi::c_void = ptr::null_mut();
            let r = ft_widget_validate(w, &mut err as *mut *mut core::ffi::c_void);
            assert_eq!(r, 0);
            // On success, err_out should not have been written — pointer remains null
            assert!(err.is_null());
            ft_widget_destroy(w);
        }
    }

    // ================================================================
    // Vtable / exported traits
    // ================================================================

    #[test]
    fn vtable_header_layout() {
        use core::mem::{offset_of, size_of};
        use ffier::{VtableHandle, VtableHeader};

        assert_eq!(size_of::<VtableHeader>(), 8);
        assert_eq!(offset_of!(VtableHeader, _num_slots), 0);
        assert_eq!(offset_of!(VtableHeader, _reserved1), 2);
        assert_eq!(offset_of!(VtableHeader, _reserved2), 4);
        assert_eq!(offset_of!(ProcessorVtable, _num_slots), 0);
        assert_eq!(offset_of!(ProcessorVtable, _reserved1), 2);
        assert_eq!(offset_of!(ProcessorVtable, _reserved2), 4);
        assert_eq!(offset_of!(ProcessorVtable, drop), size_of::<VtableHeader>());
        assert_eq!(size_of::<VtableHandle>(), 2 * size_of::<*const ()>());
    }

    #[test]
    fn empty_trait_vtable_has_no_method_slots() {
        use core::mem::{size_of, size_of_val};

        let vtable = EmptyTraitVtable {
            _num_slots: 0,
            _reserved1: 0,
            _reserved2: 0,
            drop: None,
        };
        let slot_size = size_of::<Option<unsafe extern "C" fn()>>();
        assert_eq!(
            size_of_val(&vtable),
            size_of::<ffier::VtableHeader>() + slot_size
        );
        let wrapper = VtableEmptyTrait {
            value: ffier::VtableHandle {
                vtable_ptr: &vtable as *const _ as *const core::ffi::c_void,
                user_data: ptr::null(),
            },
        };
        drop(wrapper);

        let retired = RetiredTraitVtable {
            _num_slots: 3,
            _reserved1: 0,
            _reserved2: 0,
            drop: None,
            __reserved_0: None,
            __reserved_1: None,
            __reserved_2: None,
        };
        assert_eq!(
            size_of_val(&retired),
            size_of::<ffier::VtableHeader>() + 4 * slot_size,
        );
    }

    static DROP_CALLED: AtomicBool = AtomicBool::new(false);

    unsafe extern "C" fn test_process(_self_data: *mut core::ffi::c_void, input: i32) -> i32 {
        input * 2
    }

    unsafe extern "C" fn test_processor_name(
        _self_data: *mut core::ffi::c_void,
    ) -> ffier::FfierBytes {
        // SAFETY: points to a static string literal — outlives the call.
        unsafe { ffier::FfierBytes::from_str("test_proc") }
    }

    unsafe extern "C" fn test_drop(_self_data: *mut core::ffi::c_void) {
        DROP_CALLED.store(true, Ordering::SeqCst);
    }

    static PROCESSOR_VTABLE: ProcessorVtable = ProcessorVtable {
        _num_slots: 2,
        _reserved1: 0,
        _reserved2: 0,
        drop: Some(test_drop),
        process: Some(test_process),
        name: Some(test_processor_name),
    };

    fn make_processor_handle(user_data: *mut core::ffi::c_void) -> *mut core::ffi::c_void {
        unsafe {
            ft_vtable_handle_new(
                VtableProcessor::TYPE_TAG,
                &PROCESSOR_VTABLE as *const _ as *const core::ffi::c_void,
                user_data as *const core::ffi::c_void,
            )
        }
    }

    #[test]
    fn vtable_dyn_dispatch_process() {
        unsafe {
            let p = ft_pipeline_new();
            let proc = make_processor_handle(ptr::null_mut());
            ft_pipeline_run(p, proc, 21);
            assert_eq!(ft_pipeline_result_count(p), 1);
            let mut last: i32 = -1;
            let mut err: *mut core::ffi::c_void = ptr::null_mut();
            let r = ft_pipeline_last_result(p, &mut last, &mut err as *mut *mut core::ffi::c_void);
            assert_eq!(r, 0);
            assert_eq!(last, 42);
            ft_pipeline_destroy(p);
        }
    }

    #[test]
    fn vtable_drop_callback() {
        unsafe {
            DROP_CALLED.store(false, Ordering::SeqCst);
            let p = ft_pipeline_new();
            let proc = make_processor_handle(ptr::null_mut());
            ft_pipeline_run(p, proc, 1);
            assert!(DROP_CALLED.load(Ordering::SeqCst));
            ft_pipeline_destroy(p);
        }
    }

    // ================================================================
    // Mixer with vtable fruit
    // ================================================================

    unsafe extern "C" fn fruit_value(self_data: *mut core::ffi::c_void) -> i32 {
        unsafe { *(self_data as *const i32) }
    }

    unsafe extern "C" fn fruit_count_tags(
        _self_data: *mut core::ffi::c_void,
        _tags: *const ffier::FfierBytes,
        tags_len: usize,
    ) -> i32 {
        // Just return the count of tags passed
        tags_len as i32
    }

    static FRUIT_VT_WITH_COUNT_TAGS: FruitVtable = FruitVtable {
        _num_slots: 4,
        _reserved1: 0,
        _reserved2: 0,
        drop: Some(fruit_drop),
        value: Some(fruit_value),
        label: None,
        try_count: None,
        count_tags: Some(fruit_count_tags),
    };

    unsafe extern "C" fn fruit_drop(_self_data: *mut core::ffi::c_void) {}

    // Static vtable variants for fruit tests — vtable must outlive handles.
    static FRUIT_VT_DROP_VALUE: FruitVtable = FruitVtable {
        _num_slots: 4,
        _reserved1: 0,
        _reserved2: 0,
        drop: Some(fruit_drop),
        value: Some(fruit_value),
        label: None,
        try_count: None,
        count_tags: None,
    };

    static FRUIT_VT_VALUE_ONLY: FruitVtable = FruitVtable {
        _num_slots: 4,
        _reserved1: 0,
        _reserved2: 0,
        drop: None,
        value: Some(fruit_value),
        label: None,
        try_count: None,
        count_tags: None,
    };

    fn make_fruit_handle<T>(
        user_data: *mut core::ffi::c_void,
        vtable: &'static T,
    ) -> *mut core::ffi::c_void {
        unsafe {
            ft_vtable_handle_new(
                VtableFruit::TYPE_TAG,
                vtable as *const _ as *const core::ffi::c_void,
                user_data as *const core::ffi::c_void,
            )
        }
    }

    fn make_weighable_handle(
        user_data: *mut core::ffi::c_void,
        vtable: &'static WeighableVtable,
    ) -> *mut core::ffi::c_void {
        unsafe {
            ft_vtable_handle_new(
                VtableWeighable::TYPE_TAG,
                vtable as *const _ as *const core::ffi::c_void,
                user_data as *const core::ffi::c_void,
            )
        }
    }

    #[test]
    fn mixer_blend_concrete() {
        unsafe {
            let m = ft_mixer_new();
            let apple = ft_apple_new(10);
            let orange = ft_orange_new(20);
            assert_eq!(ft_mixer_blend_concrete(m, apple, orange,), 30);
            assert_eq!(ft_mixer_total(m), 30);
            ft_mixer_destroy(m);
        }
    }

    #[test]
    fn mixer_blend_hybrid() {
        unsafe {
            let m = ft_mixer_new();
            let apple = ft_apple_new(5);
            let banana = ft_banana_new(15);
            assert_eq!(ft_mixer_blend_hybrid(m, apple, banana,), 20);
            assert_eq!(ft_mixer_total(m), 20);
            ft_mixer_destroy(m);
        }
    }

    #[test]
    fn mixer_blend_dynamic() {
        unsafe {
            let m = ft_mixer_new();
            let mango = ft_mango_new(3);
            let lemon = ft_lemon_new(7);
            assert_eq!(ft_mixer_blend_dynamic(m, mango, lemon,), 10);
            assert_eq!(ft_mixer_total(m), 10);
            ft_mixer_destroy(m);
        }
    }

    // ================================================================
    // Lifetime-parameterized types
    // ================================================================

    #[test]
    fn lifetime_type_borrowing_handle() {
        unsafe {
            let w = ft_widget_new();
            ft_widget_set_count(w, 77);
            let v = ft_view_create(w);
            assert!(!v.is_null());
            assert_eq!(ft_view_source_count(v), 77);
            ft_view_destroy(v);
            ft_widget_destroy(w);
        }
    }

    #[test]
    fn lifetime_type_reading_through_borrow() {
        unsafe {
            let w = ft_widget_new();
            ft_widget_set_count(w, 123);
            let v = ft_view_create(w);
            assert_eq!(ft_view_source_count(v), 123);
            ft_view_destroy(v);
            ft_widget_destroy(w);
        }
    }

    #[test]
    fn lifetime_type_str_methods() {
        unsafe {
            let w = ft_widget_new();
            let v = ft_view_create(w);
            assert_eq!(ft_view_label(v).as_str_unchecked(), "default");
            ft_view_set_label(v, ffier::FfierBytes::from_str("custom"));
            assert_eq!(ft_view_label(v).as_str_unchecked(), "custom");
            ft_view_destroy(v);
            ft_widget_destroy(w);
        }
    }

    // ================================================================
    // Destroy
    // ================================================================

    #[test]
    fn destroy_handle() {
        unsafe {
            let w = ft_widget_new();
            ft_widget_destroy(w);
        }
    }

    #[test]
    fn destroy_null_handle() {
        unsafe { ft_widget_destroy(ptr::null_mut()) };
    }

    // ================================================================
    // Self-dispatch — trait-scoped functions that dispatch by type tag
    // ================================================================

    #[test]
    fn self_dispatch_fruit_value_on_apple() {
        unsafe {
            let apple = ft_apple_new(42);
            // ft_fruit_value dispatches to Apple::value via type tag
            assert_eq!(ft_fruit_value(apple), 42);
            ft_apple_destroy(apple);
        }
    }

    #[test]
    fn self_dispatch_fruit_value_on_orange() {
        unsafe {
            let orange = ft_orange_new(99);
            assert_eq!(ft_fruit_value(orange), 99);
            ft_orange_destroy(orange);
        }
    }

    #[test]
    fn self_dispatch_fruit_value_on_vtable_fruit() {
        unsafe {
            // Create a VtableFruit via the vtable mechanism.
            // fruit_value dereferences self_data as *const i32.
            let val: i32 = 77;
            let handle = make_fruit_handle(
                &val as *const i32 as *mut core::ffi::c_void,
                &FRUIT_VT_DROP_VALUE,
            );
            assert_eq!(ft_fruit_value(handle), 77);
            ft_fruit_destroy(handle);
        }
    }

    #[test]
    fn self_dispatch_count_tags_on_apple() {
        unsafe {
            let apple = ft_apple_new(10);
            let tags = [
                ffier::FfierBytes::from_str("a"),
                ffier::FfierBytes::from_str("b"),
                ffier::FfierBytes::from_str("c"),
            ];
            // Apple::count_tags returns tags.len() + self.weight = 3 + 10 = 13
            assert_eq!(ft_fruit_count_tags(apple, tags.as_ptr(), tags.len()), 13);
            ft_apple_destroy(apple);
        }
    }

    #[test]
    fn vtable_count_tags() {
        unsafe {
            let handle = make_fruit_handle(ptr::null_mut(), &FRUIT_VT_WITH_COUNT_TAGS);
            let tags = [
                ffier::FfierBytes::from_str("x"),
                ffier::FfierBytes::from_str("y"),
            ];
            // VtableFruit dispatches through C fn ptr, which returns tags_len = 2
            assert_eq!(ft_fruit_count_tags(handle, tags.as_ptr(), tags.len()), 2);
            ft_fruit_destroy(handle);
        }
    }

    #[test]
    fn self_dispatch_fruit_destroy_on_apple() {
        unsafe {
            let apple = ft_apple_new(1);
            // ft_fruit_destroy dispatches to the right destructor
            ft_fruit_destroy(apple);
        }
    }

    #[test]
    fn manual_object_array_borrow_chain() {
        unsafe {
            // Create a widget that owns gadgets, get the array via FFI
            let w = ft_widget_new();
            let arr = ft_widget_gadgets(w);
            assert_eq!(arr.len, 2);

            // Get element handles from the array
            let h0 = ffier::ffier_object_array_get(arr, 0);
            let h1 = ffier::ffier_object_array_get(arr, 1);

            // Verify the borrow chain: h0/h1 are FfierBorrowedHandle
            // with METADATA_INDIRECT and no ffier allocation. Reading
            // through them does ptr hop: FfierBorrowedHandle.ptr → &Gadget.
            assert_eq!(ft_gadget_get(h0), 10);
            assert_eq!(ft_gadget_get(h1), 20);

            // Now manually construct a SECOND array reusing the same
            // borrowed handle pointers but in reverse order — verifies
            // the ptr hop works on manually constructed arrays too.
            let tag = ffier::handle_type_tag(h0);
            let meta = ffier::METADATA_INDIRECT;
            let ptr0 = (*(h0 as *const ffier::FfierBorrowedHandle)).ptr;
            let ptr1 = (*(h1 as *const ffier::FfierBorrowedHandle)).ptr;

            let reversed: Box<[ffier::FfierBorrowedHandle]> = vec![
                ffier::FfierBorrowedHandle {
                    type_tag: tag,
                    metadata: meta,
                    ptr: ptr1,
                },
                ffier::FfierBorrowedHandle {
                    type_tag: tag,
                    metadata: meta,
                    ptr: ptr0,
                },
            ]
            .into_boxed_slice();
            let len = reversed.len();
            let raw = Box::into_raw(reversed) as *const ffier::FfierBorrowedHandle;
            let arr2 = ffier::FfierObjectArray::from_raw(raw, len);

            // Read reversed array — ptr hop should give 20, 10
            let r0 = ffier::ffier_object_array_get(arr2, 0);
            let r1 = ffier::ffier_object_array_get(arr2, 1);
            assert_eq!(ft_gadget_get(r0), 20);
            assert_eq!(ft_gadget_get(r1), 10);

            ffier::ffier_object_array_free(arr2);
            ffier::ffier_object_array_free(arr);
            ft_widget_destroy(w);
        }
    }

    #[test]
    fn self_dispatch_processor_process() {
        unsafe {
            let handle = make_processor_handle(ptr::null_mut());
            // ft_processor_process dispatches via type tag
            assert_eq!(ft_processor_process(handle, 10), 20);
            ft_processor_destroy(handle);
        }
    }

    #[test]
    fn self_dispatch_processor_name() {
        unsafe {
            let handle = make_processor_handle(ptr::null_mut());
            assert_eq!(ft_processor_name(handle).as_str_unchecked(), "test_proc",);
            ft_processor_destroy(handle);
        }
    }

    // ================================================================
    // Vtable default method fallback
    // ================================================================

    #[test]
    fn vtable_default_method_uses_fallback() {
        unsafe {
            // VtableFruit with label = None → should use the default "fruit"
            let handle = make_fruit_handle(ptr::null_mut(), &FRUIT_VT_DROP_VALUE);
            assert_eq!(ft_fruit_label(handle).as_str_unchecked(), "fruit");
            ft_fruit_destroy(handle);
        }
    }

    unsafe extern "C" fn custom_label(_self_data: *mut core::ffi::c_void) -> ffier::FfierBytes {
        unsafe { ffier::FfierBytes::from_str("custom") }
    }

    static FRUIT_VT_CUSTOM_LABEL: FruitVtable = FruitVtable {
        _num_slots: 4,
        _reserved1: 0,
        _reserved2: 0,
        drop: Some(fruit_drop),
        value: Some(fruit_value),
        label: Some(custom_label),
        try_count: None,
        count_tags: None,
    };

    #[test]
    fn vtable_default_method_overridden() {
        unsafe {
            // VtableFruit with label = Some(custom) → should use the custom impl
            let handle = make_fruit_handle(ptr::null_mut(), &FRUIT_VT_CUSTOM_LABEL);
            assert_eq!(ft_fruit_label(handle).as_str_unchecked(), "custom");
            ft_fruit_destroy(handle);
        }
    }

    #[test]
    fn self_dispatch_default_method_on_concrete_type() {
        unsafe {
            // Apple doesn't override label → default "fruit" via self-dispatch
            let apple = ft_apple_new(10);
            assert_eq!(ft_fruit_label(apple).as_str_unchecked(), "fruit");
            ft_apple_destroy(apple);
        }
    }

    // ================================================================
    // Vtable forward/backward compatibility
    // ================================================================

    #[test]
    fn vtable_smaller_than_expected_uses_defaults() {
        // An older client's actual allocation ends after drop and value.
        #[repr(C)]
        struct ShortFruitVtable {
            header: ffier::VtableHeader,
            drop: Option<unsafe extern "C" fn(*mut core::ffi::c_void)>,
            value: Option<unsafe extern "C" fn(*mut core::ffi::c_void) -> i32>,
        }
        static VTABLE: ShortFruitVtable = ShortFruitVtable {
            header: ffier::VtableHeader {
                _num_slots: 1,
                _reserved1: 0,
                _reserved2: 0,
            },
            drop: Some(fruit_drop),
            value: Some(fruit_value),
        };
        unsafe {
            let val: i32 = 42;
            let handle = make_fruit_handle(&val as *const i32 as *mut core::ffi::c_void, &VTABLE);
            // The absent label slot uses the default without reading past VTABLE.
            assert_eq!(ft_fruit_label(handle).as_str_unchecked(), "fruit");
            assert_eq!(ft_fruit_value(handle), 42);
            ft_fruit_destroy(handle);
        }
    }

    #[test]
    fn vtable_larger_than_expected_works() {
        #[repr(C)]
        struct LongFruitVtable {
            fruit: FruitVtable,
            extra: [Option<unsafe extern "C" fn()>; 8],
        }
        static VTABLE: LongFruitVtable = LongFruitVtable {
            fruit: FruitVtable {
                _num_slots: 12,
                ..FRUIT_VT_CUSTOM_LABEL
            },
            extra: [None; 8],
        };
        unsafe {
            let val: i32 = 42;
            let handle = make_fruit_handle(&val as *const i32 as *mut core::ffi::c_void, &VTABLE);
            // All known fields work normally
            assert_eq!(ft_fruit_label(handle).as_str_unchecked(), "custom");
            assert_eq!(ft_fruit_value(handle), 42);
            ft_fruit_destroy(handle);
        }
    }

    #[test]
    fn vtable_zero_len_all_defaults() {
        // Drop is always present, even without any virtual method slots.
        #[repr(C)]
        struct DropOnlyVtable {
            header: ffier::VtableHeader,
            drop: Option<unsafe extern "C" fn(*mut core::ffi::c_void)>,
        }
        unsafe extern "C" fn mark_dropped(user_data: *mut core::ffi::c_void) {
            unsafe { *(user_data as *mut bool) = true };
        }
        static VTABLES: [DropOnlyVtable; 2] = [
            DropOnlyVtable {
                header: ffier::VtableHeader {
                    _num_slots: 0,
                    _reserved1: 0,
                    _reserved2: 0,
                },
                drop: None,
            },
            DropOnlyVtable {
                header: ffier::VtableHeader {
                    _num_slots: 0,
                    _reserved1: 0,
                    _reserved2: 0,
                },
                drop: Some(mark_dropped),
            },
        ];
        for vtable in &VTABLES {
            let mut dropped = false;
            unsafe {
                let handle =
                    make_fruit_handle(&mut dropped as *mut bool as *mut core::ffi::c_void, vtable);
                assert_eq!(ft_fruit_label(handle).as_str_unchecked(), "fruit");
                assert!(!dropped);
                ft_fruit_destroy(handle);
            }
            assert_eq!(dropped, vtable.drop.is_some());
        }
    }

    // ================================================================
    // Debug: handle type inspection
    // ================================================================

    #[test]
    fn debug_handle_type_vtable_fruit() {
        unsafe {
            let handle = make_fruit_handle(ptr::null_mut(), &FRUIT_VT_VALUE_ONLY);
            assert_eq!(
                ft_debug_handle_type(handle).as_str_unchecked(),
                "VtableFruit",
            );
            ft_fruit_destroy(handle);
        }
    }

    #[test]
    fn debug_handle_type_apple() {
        unsafe {
            let apple = ft_apple_new(1);
            assert_eq!(ft_debug_handle_type(apple).as_str_unchecked(), "Apple",);
            ft_apple_destroy(apple);
        }
    }

    #[test]
    fn debug_handle_type_null() {
        unsafe {
            assert_eq!(ft_debug_handle_type(ptr::null()).as_str_unchecked(), "null",);
        }
    }

    // ================================================================
    // Debug: handle roundtrip
    // ================================================================

    #[test]
    fn debug_vtable_handle_roundtrip() {
        unsafe {
            let val: i32 = 42;
            let handle = make_fruit_handle(
                &val as *const i32 as *mut core::ffi::c_void,
                &FRUIT_VT_VALUE_ONLY,
            );

            // Verify handle is valid
            assert_eq!(
                ft_debug_handle_type(handle).as_str_unchecked(),
                "VtableFruit",
            );

            // Call ft_fruit_label — label is None, should use default
            assert_eq!(ft_fruit_label(handle).as_str_unchecked(), "fruit");

            // Call ft_fruit_value — dereferences self_data as *const i32
            assert_eq!(ft_fruit_value(handle), 42);

            ft_fruit_destroy(handle);
        }
    }

    // ================================================================
    // Enum constants + free functions
    // ================================================================

    #[test]
    fn free_fn_with_enum_param() {
        unsafe {
            // LogLevel::Info = 3
            let name = ft_log_level_name(3);
            assert_eq!(name.as_str_unchecked(), "info");
        }
    }

    #[test]
    fn free_fn_with_enum_param_off() {
        unsafe {
            // LogLevel::Off = 0
            let name = ft_log_level_name(0);
            assert_eq!(name.as_str_unchecked(), "off");
        }
    }

    #[test]
    fn free_fn_returning_bool_with_enum_param() {
        unsafe {
            // LogLevel::Off = 0 → not enabled
            assert!(!ft_log_level_is_enabled(0));
            // LogLevel::Error = 1 → enabled
            assert!(ft_log_level_is_enabled(1));
            // LogLevel::Trace = 5 → enabled
            assert!(ft_log_level_is_enabled(5));
        }
    }

    // ================================================================
    // Free function with BorrowedFd / OwnedFd
    // ================================================================

    #[test]
    fn free_fn_clone_fd() {
        use std::os::unix::io::{AsRawFd, FromRawFd, OwnedFd};
        unsafe {
            // Use stdout as a known-valid fd to clone
            let stdout_fd = std::io::stdout().as_raw_fd();
            let mut result: i32 = -1;
            let mut err_out: *mut core::ffi::c_void = core::ptr::null_mut();
            let r = ft_clone_fd(
                stdout_fd,
                &mut result as *mut i32,
                &mut err_out as *mut *mut core::ffi::c_void,
            );
            assert_eq!(r, 0, "ft_clone_fd should succeed");
            assert!(result >= 0, "cloned fd should be valid");
            // Clean up the cloned fd
            drop(OwnedFd::from_raw_fd(result));
        }
    }

    // ================================================================
    // Foreign trait (Weighable from foreign-trait-crate)
    // ================================================================

    #[test]
    fn foreign_trait_impl_via_bridge() {
        unsafe {
            let apple = ft_apple_new(150);
            // Apple.weight_grams() returns weight * 10
            assert_eq!(ft_apple_weight_grams(apple), 1500);
            ft_apple_destroy(apple);
        }
    }

    #[test]
    fn foreign_trait_self_dispatch() {
        unsafe {
            let apple = ft_apple_new(200);
            // Self-dispatch through ft_weighable_weight_grams
            assert_eq!(ft_weighable_weight_grams(apple), 2000);
            ft_weighable_destroy(apple);
        }
    }

    // -----------------------------------------------------------------------
    // Handle slice: &[&T] as param
    // -----------------------------------------------------------------------

    #[test]
    fn handle_slice_param_method() {
        unsafe {
            let w = ft_widget_new();

            // Create gadgets via widget (they have widget's count as initial value)
            ft_widget_set_count(w, 5);
            let g1 = ft_widget_create_gadget(w);
            let g2 = ft_widget_create_gadget(w);

            ft_widget_set_count(w, 7);
            let g3 = ft_widget_create_gadget(w);

            // sum_gadgets takes &[&Gadget]
            let handles = [g1, g2, g3];
            let sum = ft_widget_sum_gadgets(w, handles.as_ptr(), handles.len());
            // g1.value=5, g2.value=5, g3.value=7 → sum=17
            assert_eq!(sum, 17);

            ft_gadget_destroy(g1);
            ft_gadget_destroy(g2);
            ft_gadget_destroy(g3);
            ft_widget_destroy(w);
        }
    }

    #[test]
    fn handle_slice_param_free_function() {
        unsafe {
            let w = ft_widget_new();
            ft_widget_set_count(w, 3);
            let g1 = ft_widget_create_gadget(w);
            ft_widget_set_count(w, 4);
            let g2 = ft_widget_create_gadget(w);

            let handles = [g1, g2];
            let sum = ft_sum_gadget_values(handles.as_ptr(), handles.len());
            // g1.value=3, g2.value=4 → sum=7
            assert_eq!(sum, 7);

            ft_gadget_destroy(g1);
            ft_gadget_destroy(g2);
            ft_widget_destroy(w);
        }
    }

    #[test]
    fn handle_slice_param_empty() {
        unsafe {
            let w = ft_widget_new();
            // Empty slice
            let sum = ft_widget_sum_gadgets(w, core::ptr::null(), 0);
            assert_eq!(sum, 0);
            ft_widget_destroy(w);
        }
    }

    // -----------------------------------------------------------------------
    // Primitive slice: &[u32] as param
    // -----------------------------------------------------------------------

    #[test]
    fn primitive_slice_param_method() {
        unsafe {
            let w = ft_widget_new();
            let values: [u32; 3] = [10, 20, 30];
            let sum = ft_widget_sum_values(w, values.as_ptr(), values.len());
            assert_eq!(sum, 60);
            ft_widget_destroy(w);
        }
    }

    #[test]
    fn primitive_slice_param_empty() {
        unsafe {
            let w = ft_widget_new();
            let sum = ft_widget_sum_values(w, core::ptr::null(), 0);
            assert_eq!(sum, 0);
            ft_widget_destroy(w);
        }
    }

    // -----------------------------------------------------------------------
    // &[T] direct handle slice return
    // -----------------------------------------------------------------------

    #[test]
    fn direct_handle_slice_return() {
        unsafe {
            let w = ft_widget_new();
            let arr = ft_widget_gadgets(w);
            assert_eq!(arr.len, 2);

            // Access each element via object_array_get and read its value
            let g0 = ffier::ffier_object_array_get(arr, 0);
            let g1 = ffier::ffier_object_array_get(arr, 1);
            assert_eq!(ft_gadget_get(g0), 10);
            assert_eq!(ft_gadget_get(g1), 20);

            // Free the array (does NOT destroy the gadgets — they're borrowed)
            ffier::ffier_object_array_free(arr);
            ft_widget_destroy(w);
        }
    }

    #[test]
    fn borrowed_shell_is_destroyable_but_does_not_drop_its_value() {
        // A borrowed shell sets FFIER_ALLOCATED|INDIRECT and clears
        // DROP_VALUE: the handle may be freed, the pointee must not be
        // dropped.
        unsafe {
            let w = ft_widget_new();
            let shell = ft_widget_gadget(w);
            let meta = ffier::handle_metadata(shell);
            assert_ne!(meta & ffier::METADATA_FFIER_ALLOCATED, 0);
            assert_ne!(meta & ffier::METADATA_INDIRECT, 0);
            assert_eq!(meta & ffier::METADATA_DROP_VALUE, 0);

            // Freeing the shell is legitimate and must not touch the widget:
            // reading through a fresh shell still sees the original value.
            let expected = ft_gadget_get(shell);
            ffier::ffier_handle_drop::<Gadget>(shell);
            let again = ft_widget_gadget(w);
            assert_eq!(ft_gadget_get(again), expected);
            ffier::ffier_handle_drop::<Gadget>(again);
            ft_widget_destroy(w);
        }
    }

    #[test]
    fn indirect_handle_with_drop_value_is_rejected() {
        // Owning a pointee behind a pointer is not implemented: dropping such a
        // handle would free it and leak the pointee, so it must assert instead.
        unsafe {
            let w = ft_widget_new();
            let shell = ft_widget_gadget(w);
            let meta = ffier::handle_metadata(shell) | ffier::METADATA_DROP_VALUE;
            // Rebuild the header with the impossible pair, keeping the payload.
            let borrowed = &*(shell as *const ffier::FfierBorrowedHandle);
            let fake = ffier::FfierBorrowedHandle {
                type_tag: borrowed.type_tag,
                metadata: meta,
                ptr: borrowed.ptr,
            };
            let rejected = std::panic::catch_unwind(|| unsafe {
                ffier::ffier_handle_drop::<Gadget>(&fake as *const _ as *mut core::ffi::c_void)
            });
            assert!(rejected.is_err(), "indirect drop must not be accepted");

            // The real shell still has to be released.
            ffier::ffier_handle_drop::<Gadget>(shell);
            ft_widget_destroy(w);
        }
    }

    #[test]
    fn object_array_element_is_not_individually_destroyable() {
        // Array elements are indirect without a ffier allocation, which is what
        // distinguishes them from an individually freeable borrowed handle.
        unsafe {
            let w = ft_widget_new();
            let arr = ft_widget_gadgets(w);
            assert_eq!(arr.len, 2);

            let element = ffier::ffier_object_array_get(arr, 0);
            let meta = ffier::handle_metadata(element);
            assert_ne!(meta & ffier::METADATA_INDIRECT, 0);
            assert_eq!(meta & ffier::METADATA_FFIER_ALLOCATED, 0);

            // Call the runtime directly: the extern "C" wrapper cannot unwind,
            // so a panic there would abort instead of being observable.
            let destroyed =
                std::panic::catch_unwind(|| unsafe { ffier::ffier_handle_drop::<Gadget>(element) });
            assert!(
                destroyed.is_err(),
                "destroying an array element must be rejected"
            );

            ffier::ffier_object_array_free(arr);
            ft_widget_destroy(w);
        }
    }

    #[test]
    fn foreign_trait_vtable_dispatch() {
        // Implement Weighable via vtable from C side
        unsafe extern "C" fn custom_weight(self_data: *mut core::ffi::c_void) -> i32 {
            unsafe { *(self_data as *const i32) }
        }

        static WEIGHABLE_VT: WeighableVtable = WeighableVtable {
            _num_slots: 1,
            _reserved1: 0,
            _reserved2: 0,
            drop: None,
            weight_grams: Some(custom_weight),
        };

        unsafe {
            let val: i32 = 77;
            let handle =
                make_weighable_handle(&val as *const i32 as *mut core::ffi::c_void, &WEIGHABLE_VT);

            // Self-dispatch should route through vtable
            assert_eq!(ft_weighable_weight_grams(handle), 77);

            ft_weighable_destroy(handle);
        }
    }

    // -----------------------------------------------------------------------
    // Opaque raw pointer tests — *mut c_void / *const c_void passthrough
    // -----------------------------------------------------------------------

    #[test]
    fn opaque_ptr_round_trip() {
        unsafe {
            let val: i32 = 0xDEAD;
            let ptr = &val as *const i32 as *mut core::ffi::c_void;
            let result = ft_opaque_round_trip(ptr);
            assert_eq!(result, ptr);
        }
    }

    #[test]
    fn opaque_ptr_null_round_trip() {
        unsafe {
            let result = ft_opaque_round_trip(core::ptr::null_mut());
            assert!(result.is_null());
        }
    }

    #[test]
    fn opaque_const_ptr_to_int() {
        unsafe {
            let val: i32 = 42;
            let ptr = &val as *const i32 as *const core::ffi::c_void;
            let addr = ft_opaque_ptr_to_int(ptr);
            assert_eq!(addr, ptr as usize);
        }
    }

    // -----------------------------------------------------------------------
    // no_vtable marker trait — dispatch through concrete types only
    // -----------------------------------------------------------------------

    #[test]
    fn no_vtable_dispatch_apple() {
        unsafe {
            // impl Categorizable is by-value — consumes the handle
            let a = ft_apple_new(7);
            assert_eq!(ft_categorizable_value(a), 7);
        }
    }

    #[test]
    fn no_vtable_dispatch_orange() {
        unsafe {
            let o = ft_orange_new(13);
            assert_eq!(ft_categorizable_value(o), 13);
        }
    }
}
