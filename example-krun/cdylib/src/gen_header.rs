use krun_lib::{BlockDevice, BlockDeviceBuilder, NetDevice, NetDeviceBuilder, VmResources, Vmm};

krun_lib::vmresources_ffier!(VmResources);
krun_lib::netdevice_ffier!(NetDevice<'static>);
krun_lib::blockdevice_ffier!(BlockDevice<'static>);
krun_lib::netdevicebuilder_ffier!(NetDeviceBuilder);
krun_lib::blockdevicebuilder_ffier!(BlockDeviceBuilder);
krun_lib::vmm_ffier!(Vmm<'static>);
krun_lib::vtabledevice_ffier!();

fn main() {
    // Order: dependencies first
    print!("{}", krun_vmresources__header());
    println!();
    print!("{}", krun_netdevice__header());
    println!();
    print!("{}", krun_blockdevice__header());
    println!();
    print!("{}", krun_netdevicebuilder__header());
    println!();
    print!("{}", krun_blockdevicebuilder__header());
    println!();
    print!("{}", krun_vtabledevice__header());
    println!();
    print!("{}", krun_vmm__header());
}
