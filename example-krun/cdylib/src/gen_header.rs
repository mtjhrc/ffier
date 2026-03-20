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

fn emit_common_prelude() {
    println!("#ifndef KRUN_H");
    println!("#define KRUN_H");
    println!();
    println!("#include <stdint.h>");
    println!("#include <stdbool.h>");
    println!("#include <string.h>");
    println!();
    println!("/* Opaque handles */");
    println!("typedef void* KrunVmResources;");
    println!("typedef void* KrunRootFs;");
    println!("typedef void* KrunInitPayload;");
    println!("typedef void* KrunInitPayloadBuilder;");
    println!("typedef void* KrunNetDevice;");
    println!("typedef void* KrunBlockDevice;");
    println!("typedef void* KrunNetDeviceBuilder;");
    println!("typedef void* KrunBlockDeviceBuilder;");
    println!("typedef void* KrunVmm;");
    println!("typedef void* KrunDevice;");
    println!();
    println!("/* Shared byte/string ABI types */");
    println!("/* Caller must ensure data is valid UTF-8 */");
    println!("typedef struct {{");
    println!("    const char* data;");
    println!("    uintptr_t len;");
    println!("}} KrunStr;");
    println!();
    println!("/* Caller must ensure data is a valid UTF-8 path */");
    println!("typedef KrunStr KrunPath;");
    println!();
    println!("typedef struct {{");
    println!("    const uint8_t* data;");
    println!("    uintptr_t len;");
    println!("}} KrunBytes;");
    println!();
    println!("#define KRUN_STR(s) ((KrunStr){{ .data = (s), .len = strlen(s) }})");
    println!("#define KRUN_BYTES(arr) ({{ \\");
    println!("    _Static_assert( \\");
    println!("        !__builtin_types_compatible_p(typeof(arr), typeof(&(arr)[0])), \\");
    println!("        \"KRUN_BYTES() requires an array, not a pointer\"); \\");
    println!("    ((KrunBytes){{ .data = (const uint8_t*)(arr), .len = sizeof(arr) }}); \\");
    println!("}})");
    println!();
}

fn strip_header_noise(header: &str) -> String {
    let mut out = Vec::new();
    let mut skip_bytes_block = false;

    for line in header.lines() {
        let trimmed = line.trim();

        if trimmed == "#ifndef KRUN_BYTES_DEFINED" {
            skip_bytes_block = true;
            continue;
        }
        if skip_bytes_block {
            if trimmed == "#endif /* KRUN_BYTES_DEFINED */" {
                skip_bytes_block = false;
            }
            continue;
        }

        if trimmed.starts_with("#ifndef ")
            || trimmed.starts_with("#define ")
            || trimmed.starts_with("#endif ")
            || trimmed.starts_with("#include <")
        {
            continue;
        }

        if matches!(
            trimmed,
            "typedef void* KrunVmResources;"
                | "typedef void* KrunRootFs;"
                | "typedef void* KrunInitPayload;"
                | "typedef void* KrunInitPayloadBuilder;"
                | "typedef void* KrunNetDevice;"
                | "typedef void* KrunBlockDevice;"
                | "typedef void* KrunNetDeviceBuilder;"
                | "typedef void* KrunBlockDeviceBuilder;"
                | "typedef void* KrunVmm;"
                | "typedef void* KrunDevice;"
        ) {
            continue;
        }

        out.push(line);
    }

    let mut cleaned = String::new();
    let mut prev_blank = true;
    for line in out {
        let is_blank = line.trim().is_empty();
        if is_blank && prev_blank {
            continue;
        }
        cleaned.push_str(line);
        cleaned.push('\n');
        prev_blank = is_blank;
    }

    cleaned.trim().to_string()
}

fn emit_section(header: &str) {
    let cleaned = strip_header_noise(header);
    if !cleaned.is_empty() {
        println!("{cleaned}");
        println!();
    }
}

fn main() {
    emit_common_prelude();

    emit_section(&krun_vmresources__header());
    emit_section(&krun_rootfs__header());
    emit_section(&krun_initpayload__header());
    emit_section(&krun_initpayloadbuilder__header());
    emit_section(&krun_netdevice__header());
    emit_section(&krun_blockdevice__header());
    emit_section(&krun_netdevicebuilder__header());
    emit_section(&krun_blockdevicebuilder__header());
    emit_section(&krun_vtabledevice__header());
    emit_section(&krun_vmm__header());

    println!("#endif /* KRUN_H */");
}
