use std::os::unix::io::{AsRawFd, BorrowedFd, OwnedFd};
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, ffier::FfiError)]
pub enum TestError {
    #[ffier(code = 1)]
    NotFound,
    #[ffier(code = 2, message = "custom error message")]
    CustomMessage,
    #[ffier(code = 3)]
    InvalidInput,
}

// ---------------------------------------------------------------------------
// Widget — primary type exercising most param/return patterns
// ---------------------------------------------------------------------------

pub struct Widget {
    count: i32,
    name: String,
    active: bool,
    tags: String,
}

#[ffier::exportable]
impl Widget {
    /// Create a new widget with default values.
    pub fn new() -> Self {
        Widget {
            count: 0,
            name: String::from("widget"),
            active: true,
            tags: String::new(),
        }
    }

    /// Create a widget with a given name.
    pub fn with_name(name: &str) -> Self {
        Widget {
            count: 0,
            name: name.to_owned(),
            active: true,
            tags: String::new(),
        }
    }

    /// Get the current count.
    pub fn get_count(&self) -> i32 {
        self.count
    }

    /// Set the count.
    pub fn set_count(&mut self, n: i32) {
        self.count = n;
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

    /// Validate internal state (always succeeds for default widget).
    pub fn validate(&self) -> Result<(), TestError> {
        if self.count >= 0 {
            Ok(())
        } else {
            Err(TestError::InvalidInput)
        }
    }

    /// Parse a count value from the name length, returning error if name matches trigger.
    pub fn parse_count(&self, s: &str) -> Result<i32, TestError> {
        if s == "error" {
            Err(TestError::NotFound)
        } else {
            Ok(s.len() as i32)
        }
    }

    /// Describe a code as a string.
    pub fn describe(&self, code: i32) -> Result<&str, TestError> {
        match code {
            0 => Ok("zero"),
            1 => Ok("one"),
            _ => Err(TestError::NotFound),
        }
    }

    /// Always fails with an error.
    pub fn fail_always(&self) -> Result<(), TestError> {
        Err(TestError::CustomMessage)
    }

    /// Always fails with an error (value variant).
    pub fn fail_with_value(&self) -> Result<i32, TestError> {
        Err(TestError::InvalidInput)
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

    /// Try to create a gadget; fails if ok is false.
    pub fn try_create_gadget(&self, ok: bool) -> Result<Gadget, TestError> {
        if ok {
            Ok(Gadget { value: self.count })
        } else {
            Err(TestError::NotFound)
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

    /// Consume the widget (by-value self, void return).
    pub fn consume(self) {
        drop(self);
    }

    /// Get the raw fd number from a borrowed fd.
    pub fn fd_number(&self, fd: BorrowedFd<'_>) -> i32 {
        fd.as_raw_fd()
    }

    /// Duplicate a file descriptor (returns owned fd).
    pub fn dup_fd(&self, fd: BorrowedFd<'_>) -> OwnedFd {
        fd.try_clone_to_owned().expect("dup failed")
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

#[ffier::exportable]
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

#[ffier::exportable]
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
            Err(TestError::InvalidInput)
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

#[ffier::exportable]
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

#[ffier::exportable]
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
            Err(TestError::InvalidInput)
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

#[ffier::exportable]
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

#[ffier::exportable]
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
// Implementable trait: Processor (with supertrait Observer)
// ---------------------------------------------------------------------------

pub trait Observer {
    fn on_notify(&self, code: i32);
}

#[ffier::implementable(
    supers(Observer { fn on_notify(&self, code: i32); })
)]
pub trait Processor: Observer {
    fn process(&self, input: i32) -> i32;
    fn name(&self) -> &str;
}

pub trait IntoProcessor {
    fn into_processor(self) -> Arc<dyn Processor>;
}

impl<T: Processor + 'static> IntoProcessor for T {
    fn into_processor(self) -> Arc<dyn Processor> {
        Arc::new(self)
    }
}

// ---------------------------------------------------------------------------
// Pipeline — uses dyn_param with Processor
// ---------------------------------------------------------------------------

pub struct Pipeline {
    results: Vec<i32>,
}

#[ffier::exportable]
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
        proc.on_notify(result);
        self.results.push(result);
    }

    /// Get the number of results.
    pub fn result_count(&self) -> i32 {
        self.results.len() as i32
    }

    /// Get the last result, or error if empty.
    pub fn last_result(&self) -> Result<i32, TestError> {
        self.results.last().copied().ok_or(TestError::NotFound)
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

#[ffier::exportable]
impl Apple {
    pub fn new(weight: i32) -> Self {
        Apple { weight }
    }
}

pub struct Orange {
    juice: i32,
}

#[ffier::exportable]
impl Orange {
    pub fn new(juice: i32) -> Self {
        Orange { juice }
    }
}

#[ffier::implementable]
pub trait Fruit {
    fn value(&self) -> i32;
}

#[ffier::trait_impl]
impl Fruit for Apple {
    fn value(&self) -> i32 {
        self.weight
    }
}

#[ffier::trait_impl]
impl Fruit for Orange {
    fn value(&self) -> i32 {
        self.juice
    }
}

// Extra fruit types so that blend(a: impl Fruit, b: impl Fruit) has
// 9 variants (8 concrete + VtableFruit). 9^2 = 81 > 64 dispatch limit.
pub struct Banana(i32);
#[ffier::exportable]
impl Banana { pub fn new(v: i32) -> Self { Banana(v) } }
#[ffier::trait_impl]
impl Fruit for Banana { fn value(&self) -> i32 { self.0 } }

pub struct Mango(i32);
#[ffier::exportable]
impl Mango { pub fn new(v: i32) -> Self { Mango(v) } }
#[ffier::trait_impl]
impl Fruit for Mango { fn value(&self) -> i32 { self.0 } }

pub struct Peach(i32);
#[ffier::exportable]
impl Peach { pub fn new(v: i32) -> Self { Peach(v) } }
#[ffier::trait_impl]
impl Fruit for Peach { fn value(&self) -> i32 { self.0 } }

pub struct Plum(i32);
#[ffier::exportable]
impl Plum { pub fn new(v: i32) -> Self { Plum(v) } }
#[ffier::trait_impl]
impl Fruit for Plum { fn value(&self) -> i32 { self.0 } }

pub struct Grape(i32);
#[ffier::exportable]
impl Grape { pub fn new(v: i32) -> Self { Grape(v) } }
#[ffier::trait_impl]
impl Fruit for Grape { fn value(&self) -> i32 { self.0 } }

pub struct Lemon(i32);
#[ffier::exportable]
impl Lemon { pub fn new(v: i32) -> Self { Lemon(v) } }
#[ffier::trait_impl]
impl Fruit for Lemon { fn value(&self) -> i32 { self.0 } }

pub struct Mixer {
    total: i32,
}

#[ffier::exportable]
impl Mixer {
    pub fn new() -> Self {
        Mixer { total: 0 }
    }

    pub fn add(mut self, fruit: impl Fruit) -> Self {
        self.total += fruit.value();
        self
    }

    /// Both concrete (9^2=81 > 64, override with annotation).
    pub fn blend_concrete(&mut self, #[ffier(dispatch = concrete)] a: impl Fruit, #[ffier(dispatch = concrete)] b: impl Fruit) -> i32 {
        let sum = a.value() + b.value();
        self.total += sum;
        sum
    }

    /// First concrete, second vtable (hybrid: 9+9=18 branches).
    pub fn blend_hybrid(&mut self, a: impl Fruit, #[ffier(dispatch = vtable)] b: impl Fruit) -> i32 {
        let sum = a.value() + b.value();
        self.total += sum;
        sum
    }

    /// Both vtable (9+9=18 branches).
    pub fn blend_dynamic(&mut self, #[ffier(dispatch = vtable)] a: impl Fruit, #[ffier(dispatch = vtable)] b: impl Fruit) -> i32 {
        let sum = a.value() + b.value();
        self.total += sum;
        sum
    }

    /// Peek via generic ref (concrete dispatch, borrow).
    pub fn peek<F: Fruit>(&self, fruit: &F) -> i32 {
        fruit.value()
    }

    /// Peek via dyn ref (auto dyn coerce, no concrete branching).
    pub fn peek_dyn(&self, fruit: &dyn Fruit) -> i32 {
        fruit.value()
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

#[ffier::exportable]
impl Sprocket {
    pub fn new(name: &str) -> Self {
        Sprocket {
            name: name.to_owned(),
        }
    }
}

#[ffier::trait_impl]
impl Attachment for Sprocket {
    fn label(&self) -> &str {
        &self.name
    }

    #[ffier(skip)]
    fn attach(&self, state: &InternalState) -> bool {
        !state._data.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Lifetime-parameterized trait_impl — tests that `impl<'a> Trait<'a> for Struct<'a>`
// preserves lifetimes in generated client code.
// ---------------------------------------------------------------------------

pub trait Snapshot<'a> {
    fn snap_description(&self) -> &str;
    fn snap_source_count(&self) -> i32;
}

#[ffier::trait_impl]
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
#[ffier::trait_impl]
impl Snapshot<'static> for Widget {
    fn snap_description(&self) -> &str {
        &self.name
    }

    fn snap_source_count(&self) -> i32 {
        self.count
    }
}

// ---------------------------------------------------------------------------
// Library metadata — lists all exported types for batched generation
// ---------------------------------------------------------------------------

ffier::library_definition!("ft",
    TestError,
    Widget, Gadget, Config,
    Gizmo, GizmoBuilder,
    View, ViewFactory,
    Pipeline,
    trait Processor,
    Apple, Orange, Banana, Mango, Peach, Plum, Grape, Lemon,
    trait Fruit,
    Fruit for Apple,
    Fruit for Orange,
    Fruit for Banana,
    Fruit for Mango,
    Fruit for Peach,
    Fruit for Plum,
    Fruit for Grape,
    Fruit for Lemon,
    Mixer,
    Sprocket,
    Attachment for Sprocket,
    Snapshot for View,
    Snapshot for Widget,
);
