use krun_lib::{
    BlockDevice, BlockDeviceBuilder, InitPayload, InitPayloadBuilder, NetDevice, NetDeviceBuilder,
    RootFs, VmResources, Vmm,
};

krun_lib::vm_resources_ffier!(VmResources);
krun_lib::root_fs_ffier!(RootFs);
krun_lib::init_payload_ffier!(InitPayload<'static>);
krun_lib::init_payload_builder_ffier!(InitPayloadBuilder<'static>);
krun_lib::net_device_ffier!(NetDevice<'static>);
krun_lib::block_device_ffier!(BlockDevice<'static>);
krun_lib::net_device_builder_ffier!(NetDeviceBuilder);
krun_lib::block_device_builder_ffier!(BlockDeviceBuilder);
krun_lib::vmm_ffier!(Vmm<'static>);
krun_lib::vtable_device_ffier!();
