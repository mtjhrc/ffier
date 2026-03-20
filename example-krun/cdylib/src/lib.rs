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
