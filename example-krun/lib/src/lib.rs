use std::sync::Arc;

#[ffier::implementable(
    supers(EventSubscriber { fn on_event(&self); })
)]
pub trait Device<'a>: EventSubscriber {
    fn name(&self) -> &str;
    fn data(&self) -> &'a [u8];
}

pub trait EventSubscriber {
    fn on_event(&self);
}

pub trait IntoDevice<'a> {
    fn into_device(self) -> Arc<dyn Device<'a> + 'a>;
}

impl<'a, T: Device<'a> + 'a> IntoDevice<'a> for T {
    fn into_device(self) -> Arc<dyn Device<'a> + 'a> {
        Arc::new(self)
    }
}

impl<'a, T: Device<'a> + 'a> IntoDevice<'a> for Arc<T> {
    fn into_device(self) -> Arc<dyn Device<'a> + 'a> {
        self
    }
}

pub struct MmioTransport<'a> {
    _device: Arc<dyn Device<'a> + 'a>,
}

pub struct VmResources {
    irq_table: Vec<u8>,
    shared_mem: Vec<u8>,
}

#[ffier::exportable]
impl VmResources {
    pub fn new() -> Self {
        VmResources {
            irq_table: Vec::new(),
            shared_mem: Vec::new(),
        }
    }
}

impl Default for VmResources {
    fn default() -> Self {
        Self::new()
    }
}

pub struct RootFs {
    tag: String,
    has_injected_init: bool,
}

#[ffier::exportable]
impl RootFs {
    pub fn new(tag: &str) -> Self {
        Self {
            tag: tag.to_owned(),
            has_injected_init: false,
        }
    }

    pub fn tag(&self) -> &str {
        &self.tag
    }

    pub fn has_injected_init(&self) -> bool {
        self.has_injected_init
    }
}

pub struct InitPayload<'a> {
    rootfs_tag: &'a str,
    command_line: String,
}

#[ffier::exportable]
impl<'a> InitPayload<'a> {
    /// Create a payload builder that borrows an existing exported object.
    pub fn builder(rootfs: &'a mut RootFs) -> InitPayloadBuilder<'a> {
        InitPayloadBuilder {
            rootfs,
            command_line: String::new(),
        }
    }

    pub fn rootfs_tag(&self) -> &str {
        self.rootfs_tag
    }

    pub fn command_line(&self) -> &str {
        &self.command_line
    }
}

pub struct InitPayloadBuilder<'a> {
    rootfs: &'a mut RootFs,
    command_line: String,
}

#[ffier::exportable]
impl<'a> InitPayloadBuilder<'a> {
    /// Set the guest command line.
    pub fn set_exec(&mut self, exec_path: &str, args: &[&str]) {
        let extra_len: usize = args.iter().map(|arg| arg.len() + 1).sum();
        let mut command_line = String::with_capacity(exec_path.len() + extra_len);
        command_line.push_str(exec_path);
        for arg in args {
            command_line.push(' ');
            command_line.push_str(arg);
        }
        self.command_line = command_line;
    }

    /// Finalize the payload and mark the rootfs as containing injected init.
    pub fn build(self) -> InitPayload<'a> {
        let Self {
            rootfs,
            command_line,
        } = self;
        rootfs.has_injected_init = true;
        InitPayload {
            rootfs_tag: &rootfs.tag,
            command_line,
        }
    }
}

pub struct Vmm<'a> {
    _resources: &'a VmResources,
    transports: Vec<MmioTransport<'a>>,
    event_subscribers: Vec<Arc<dyn EventSubscriber + 'a>>,
}

#[ffier::exportable]
impl<'a> Vmm<'a> {
    /// Create a new VMM bound to the given resources.
    ///
    /// # Arguments
    ///
    /// * `resources` - VM resources (must outlive the VMM).
    pub fn new(resources: &'a VmResources) -> Self {
        Vmm {
            _resources: resources,
            transports: Vec::new(),
            event_subscribers: Vec::new(),
        }
    }

    /// Add a device to the VMM.
    ///
    /// # Arguments
    ///
    /// * `dev` - A device handle (NetDevice, BlockDevice, or custom vtable device).
    #[ffier(dyn_param(dev, "Device", [NetDevice<'a>, BlockDevice<'a>, VtableDevice]))]
    pub fn add_device(&mut self, dev: impl IntoDevice<'a>) {
        let dev = dev.into_device();
        let transport = MmioTransport {
            _device: Arc::clone(&dev),
        };
        self.transports.push(transport);
        self.event_subscribers.push(dev);
    }

    /// List all attached devices.
    ///
    /// # Returns
    ///
    /// Number of attached devices.
    pub fn device_count(&self) -> i32 {
        self.transports.len() as i32
    }

    /// Fire an event to all subscribers.
    pub fn fire_event(&self) {
        for sub in &self.event_subscribers {
            sub.on_event();
        }
    }
}

pub struct NetDevice<'a> {
    tap_buf: &'a [u8],
    irq_table: &'a [u8],
}

#[ffier::exportable]
impl<'a> NetDevice<'a> {}

pub struct NetDeviceBuilder;

#[ffier::exportable]
impl NetDeviceBuilder {
    /// Prepare a net device builder, reserving IRQ resources.
    pub fn new(resources: &mut VmResources) -> Self {
        resources.irq_table.extend_from_slice(&[0x01, 0x02]);
        NetDeviceBuilder
    }

    /// Build the network device.
    ///
    /// # Arguments
    ///
    /// * `tap_buf` - Tap buffer (must outlive the device).
    /// * `resources` - VM resources (must outlive the device).
    ///
    /// # Returns
    ///
    /// The constructed network device.
    pub fn build<'a>(self, tap_buf: &'a [u8], resources: &'a VmResources) -> NetDevice<'a> {
        NetDevice {
            tap_buf,
            irq_table: &resources.irq_table,
        }
    }
}

impl Default for NetDeviceBuilder {
    fn default() -> Self {
        NetDeviceBuilder
    }
}

impl<'a> Device<'a> for NetDevice<'a> {
    fn name(&self) -> &str {
        "net"
    }

    fn data(&self) -> &'a [u8] {
        self.tap_buf
    }
}

impl EventSubscriber for NetDevice<'_> {
    fn on_event(&self) {
        println!("  net: got event, irq_table={:?}", self.irq_table);
    }
}

pub struct BlockDevice<'a> {
    disk_image: &'a [u8],
    shared_mem: &'a [u8],
}

#[ffier::exportable]
impl<'a> BlockDevice<'a> {}

pub struct BlockDeviceBuilder;

#[ffier::exportable]
impl BlockDeviceBuilder {
    /// Prepare a block device builder, reserving shared memory.
    pub fn new(resources: &mut VmResources) -> Self {
        resources.shared_mem.resize(64, 0xAA);
        BlockDeviceBuilder
    }

    /// Build the block device.
    ///
    /// # Arguments
    ///
    /// * `disk_image` - Disk image data (must outlive the device).
    /// * `resources` - VM resources (must outlive the device).
    ///
    /// # Returns
    ///
    /// The constructed block device.
    pub fn build<'a>(self, disk_image: &'a [u8], resources: &'a VmResources) -> BlockDevice<'a> {
        BlockDevice {
            disk_image,
            shared_mem: &resources.shared_mem,
        }
    }
}

impl Default for BlockDeviceBuilder {
    fn default() -> Self {
        BlockDeviceBuilder
    }
}

impl<'a> Device<'a> for BlockDevice<'a> {
    fn name(&self) -> &str {
        "block"
    }

    fn data(&self) -> &'a [u8] {
        self.disk_image
    }
}

impl EventSubscriber for BlockDevice<'_> {
    fn on_event(&self) {
        println!(
            "  block: got event, shared_mem len={}",
            self.shared_mem.len()
        );
    }
}
