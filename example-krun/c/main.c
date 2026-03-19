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
    KrunVmResourcesHandle resources = krun_vmresources_new();

    /* Rust-implemented devices via builders */
    KrunNetDeviceBuilderHandle net_b = krun_netdevicebuilder_new(resources);
    KrunBlockDeviceBuilderHandle blk_b = krun_blockdevicebuilder_new(resources);

    uint8_t tap_buf[] = {0xCA, 0xFE, 0xBA, 0xBE};
    uint8_t disk_data[] = {0xDE, 0xAD, 0xBE, 0xEF, 0x00};

    KrunBytes tap = { .data = (const char*)tap_buf, .len = sizeof(tap_buf) };
    KrunBytes disk = { .data = (const char*)disk_data, .len = sizeof(disk_data) };

    KrunNetDeviceHandle net_dev = krun_netdevicebuilder_build(net_b, tap, resources);
    KrunBlockDeviceHandle blk_dev = krun_blockdevicebuilder_build(blk_b, disk, resources);

    /* C-implemented device via vtable */
    KrunDevice custom_dev = krun_device_from_vtable(NULL, &MY_DEVICE_VTABLE);

    /* Create VMM and add all three devices through ONE function */
    KrunVmmHandle vmm = krun_vmm_new(resources);
    krun_vmm_add_device(vmm, net_dev);
    krun_vmm_add_device(vmm, blk_dev);
    krun_vmm_add_device(vmm, custom_dev);

    printf("device_count = %d\n", krun_vmm_device_count(vmm));
    printf("firing event:\n");
    krun_vmm_fire_event(vmm);

    krun_vmm_destroy(vmm);
    krun_vmresources_destroy(resources);

    printf("All krun C tests passed!\n");
    return 0;
}
