#[cfg(test)]
mod tests {
    #[test]
    fn application_selects_strong_for_the_shared_client() {
        // The intermediate edge enables dynlookup, the application's edge strong.
        // No initialization is needed anywhere in the unified dependency graph.
        assert_eq!(linking_intermediate::answer(), 42);
        assert_eq!(linking_client::answer(), 42);
        let ignored_handle = Some(core::ptr::NonNull::dangling());
        unsafe {
            linking_client::set_library_handle(ignored_handle, linking_client::LoadPolicy::Lazy)
        }
        .unwrap();
        linking_client::require(&[linking_client::Symbol::FixtureMissing]).unwrap();
        assert!(linking_client::Symbol::FixtureMissing.is_available());
    }
}
