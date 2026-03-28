krun_lib::__ffier_meta_vm_resources!(ffier::generate_bridge);
krun_lib::__ffier_meta_root_fs!(ffier::generate_bridge);
krun_lib::__ffier_meta_init_payload!(ffier::generate_bridge);
krun_lib::__ffier_meta_init_payload_builder!(ffier::generate_bridge);
krun_lib::__ffier_meta_net_device!(ffier::generate_bridge);
krun_lib::__ffier_meta_block_device!(ffier::generate_bridge);
krun_lib::__ffier_meta_net_device_builder!(ffier::generate_bridge);
krun_lib::__ffier_meta_block_device_builder!(ffier::generate_bridge);
krun_lib::__ffier_meta_vmm!(ffier::generate_bridge);
krun_lib::__ffier_meta_vtable_device!(ffier::generate_bridge);

fn main() {
    let header = ffier::HeaderBuilder::new("KRUN_H")
        .add(krun_vm_resources__header())
        .add(krun_root_fs__header())
        .add(krun_init_payload__header())
        .add(krun_init_payload_builder__header())
        .add(krun_net_device__header())
        .add(krun_block_device__header())
        .add(krun_net_device_builder__header())
        .add(krun_block_device_builder__header())
        .add(krun_vtable_device__header())
        .add(krun_vmm__header())
        .build();
    print!("{header}");
}
