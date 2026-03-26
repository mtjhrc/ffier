use krun_lib::{
    BlockDevice, BlockDeviceBuilder, InitPayload, InitPayloadBuilder, NetDevice, NetDeviceBuilder,
    RootFs, VmResources, Vmm,
};
krun_lib::vmresources_ffier!(VmResources);
krun_lib::rootfs_ffier!(RootFs);
krun_lib::initpayload_ffier!(InitPayload<'static>);
krun_lib::initpayloadbuilder_ffier!(InitPayloadBuilder<'static>);
krun_lib::netdevice_ffier!(NetDevice<'static>);
krun_lib::blockdevice_ffier!(BlockDevice<'static>);
krun_lib::netdevicebuilder_ffier!(NetDeviceBuilder);
krun_lib::blockdevicebuilder_ffier!(BlockDeviceBuilder);
krun_lib::vmm_ffier!(Vmm<'static>);
krun_lib::vtabledevice_ffier!();

fn main() {    
    let header = ffier::HeaderBuilder::new("KRUN_H")
        .add(krun_vmresources__header())
        .add(krun_rootfs__header())
        .add(krun_initpayload__header())
        .add(krun_initpayloadbuilder__header())
        .add(krun_netdevice__header())
        .add(krun_blockdevice__header())
        .add(krun_netdevicebuilder__header())
        .add(krun_blockdevicebuilder__header())
        .add(krun_vtabledevice__header())
        .add(krun_vmm__header())
        .build();
    print!("{header}");
}
