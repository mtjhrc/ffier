use super::Inner;

#[ffier::export]
#[repr(C)]
pub struct Outer {
    pub inner: super::Inner,
    pub crate_inner: crate::Inner,
    pub self_inner: self::Inner,
}
