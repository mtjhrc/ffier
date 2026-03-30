krun_lib::ffier_meta_op_vm_resources!("krun", ffier::generate_bridge);
krun_lib::ffier_meta_op_root_fs!("krun", ffier::generate_bridge);
krun_lib::ffier_meta_op_init_payload!("krun", ffier::generate_bridge);
krun_lib::ffier_meta_op_init_payload_builder!("krun", ffier::generate_bridge);
krun_lib::ffier_meta_op_net_device!("krun", ffier::generate_bridge);
krun_lib::ffier_meta_op_block_device!("krun", ffier::generate_bridge);
krun_lib::ffier_meta_op_net_device_builder!("krun", ffier::generate_bridge);
krun_lib::ffier_meta_op_block_device_builder!("krun", ffier::generate_bridge);
krun_lib::ffier_meta_op_vmm!("krun", ffier::generate_bridge);
krun_lib::ffier_meta_op_vtable_device!("krun", ffier::generate_bridge);

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
