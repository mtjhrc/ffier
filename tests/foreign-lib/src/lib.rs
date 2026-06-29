// A separate ffier library (library_tag = 2) whose types will be used
// as foreign types in ffier-test-lib (library_tag = 1).

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq, thiserror::Error, ffier::FfiError)]
#[non_exhaustive]
pub enum ForeignError {
    #[error("foreign error: invalid")]
    #[ffier(code = 1)]
    Invalid(),
}

// ---------------------------------------------------------------------------
// ForeignItem — a simple data object from the "foreign" library
// ---------------------------------------------------------------------------

pub struct ForeignItem {
    pub label: String,
    pub score: i32,
}

#[ffier::export]
impl ForeignItem {
    pub fn new(label: &str, score: i32) -> Self {
        ForeignItem {
            label: label.to_owned(),
            score,
        }
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub fn score(&self) -> i32 {
        self.score
    }

    pub fn set_score(&mut self, score: i32) {
        self.score = score;
    }
}

// ---------------------------------------------------------------------------
// ForeignConfig — a config-like object
// ---------------------------------------------------------------------------

pub struct ForeignConfig {
    pub name: String,
    pub value: i32,
}

#[ffier::export]
impl ForeignConfig {
    pub fn new(name: &str, value: i32) -> Self {
        ForeignConfig {
            name: name.to_owned(),
            value,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn value(&self) -> i32 {
        self.value
    }
}

// ---------------------------------------------------------------------------
// Library definition
// ---------------------------------------------------------------------------

ffier::library_definition!("fl", library_tag = 2,
    ForeignError = 1,
    ForeignItem = 2,
    ForeignConfig = 3,
    trait ffier_builtins::PushStr = 4,
    trait ffier_builtins::Error = 5,
    Error for ForeignError,
);

ffier::generate_bridge!(
    local = __ffier_fl_metadata,
    schema_output = "../../target/ffier-fl.json"
);
