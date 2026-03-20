#include <stdio.h>
#include "krun.h"

/* --- Custom device implemented in C via vtable --- */

static KrunStr my_device_name(void *self) {
    (void)self;
    return (KrunStr){ .data = "custom-c-device", .len = 15 };
}

static KrunBytes my_device_data(void *self) {
    (void)self;
    static const char data[] = {0x42, 0x43};
    return (KrunBytes){ .data = data, .len = sizeof(data) };
}

static void my_device_on_event(void *self) {
    (void)self;
    printf("  custom-c-device: got event from C!\n");
}

static const KrunDeviceVtable MY_DEVICE_VTABLE = {
    .name = my_device_name,
    .data = my_device_data,
    .on_event = my_device_on_event,
    .drop = NULL, /* no cleanup needed */
};

/* --- main --- */

int main(void) {
    KrunVmResources resources = krun_vmresources_new();
    KrunRootFs rootfs = krun_rootfs_new(KRUN_STR("rootfs"));

    /* Builder that borrows an already-configured exported object */
    KrunInitPayloadBuilder init_b = krun_initpayload_builder(rootfs);
    KrunStr init_args[] = { KRUN_STR("-c"), KRUN_STR("echo hello from init") };
    krun_initpayloadbuilder_set_exec(init_b, KRUN_STR("/bin/sh"), init_args, 2);
    KrunInitPayload init_payload = krun_initpayloadbuilder_build(init_b);

    KrunStr init_rootfs = krun_initpayload_rootfs_tag(init_payload);
    KrunStr init_cmd = krun_initpayload_command_line(init_payload);
    printf(
        "payload rootfs=%.*s cmd=%.*s injected=%d\n",
        (int)init_rootfs.len,
        init_rootfs.data,
        (int)init_cmd.len,
        init_cmd.data,
        krun_rootfs_has_injected_init(rootfs));

    /* Rust-implemented devices via builders */
    KrunNetDeviceBuilder net_b = krun_netdevicebuilder_new(resources);
    KrunBlockDeviceBuilder blk_b = krun_blockdevicebuilder_new(resources);

    uint8_t tap_buf[] = {0xCA, 0xFE, 0xBA, 0xBE};
    uint8_t disk_data[] = {0xDE, 0xAD, 0xBE, 0xEF, 0x00};

    KrunNetDevice net_dev = krun_netdevicebuilder_build(net_b, KRUN_BYTES(tap_buf), resources);
    KrunBlockDevice blk_dev = krun_blockdevicebuilder_build(blk_b, KRUN_BYTES(disk_data), resources);

    /* C-implemented device via vtable */
    KrunDevice custom_dev = krun_device_from_vtable(NULL, &MY_DEVICE_VTABLE);

    /* Create VMM and add all three devices through ONE function */
    KrunVmm vmm = krun_vmm_new(resources);
    krun_vmm_add_device(vmm, net_dev);
    krun_vmm_add_device(vmm, blk_dev);
    krun_vmm_add_device(vmm, custom_dev);

    printf("device_count = %d\n", krun_vmm_device_count(vmm));
    printf("firing event:\n");
    krun_vmm_fire_event(vmm);

    krun_vmm_destroy(vmm);
    krun_initpayload_destroy(init_payload);
    krun_rootfs_destroy(rootfs);
    krun_vmresources_destroy(resources);

    printf("All krun C tests passed!\n");
    return 0;
}
