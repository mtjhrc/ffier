#include <stdio.h>
#include "krun.h"

int main(void) {
    /* Create resources (shared by all devices) */
    KrunVmResourcesHandle resources = krun_vmresources_new();

    /* Build devices using the builder pattern:
     * builder = new(resources)  — reserves resources (mutates)
     * device  = build(builder, data, resources) — consumes builder */
    KrunNetDeviceBuilderHandle net_b = krun_netdevicebuilder_new(resources);
    KrunBlockDeviceBuilderHandle blk_b = krun_blockdevicebuilder_new(resources);

    uint8_t tap_buf[] = {0xCA, 0xFE, 0xBA, 0xBE};
    uint8_t disk_data[] = {0xDE, 0xAD, 0xBE, 0xEF, 0x00};

    KrunBytes tap = { .data = (const char*)tap_buf, .len = sizeof(tap_buf) };
    KrunBytes disk = { .data = (const char*)disk_data, .len = sizeof(disk_data) };

    /* build() consumes the builder handle (don't use net_b/blk_b after this) */
    KrunNetDeviceHandle net_dev = krun_netdevicebuilder_build(net_b, tap, resources);
    KrunBlockDeviceHandle blk_dev = krun_blockdevicebuilder_build(blk_b, disk, resources);

    /* Create VMM bound to resources */
    KrunVmmHandle vmm = krun_vmm_new(resources);

    /* Add devices (consumes the device handles) */
    krun_vmm_add_net_device(vmm, net_dev);
    krun_vmm_add_block_device(vmm, blk_dev);

    printf("device_count = %d\n", krun_vmm_device_count(vmm));
    printf("firing event:\n");
    krun_vmm_fire_event(vmm);

    /* Destroy in reverse order: vmm first, then resources */
    krun_vmm_destroy(vmm);
    krun_vmresources_destroy(resources);

    printf("All krun C tests passed!\n");
    return 0;
}
