use seq_macro::seq;
use x86::{
    Ring,
    bits64::segmentation::Descriptor64,
    dtables::{DescriptorTablePointer, lgdt, lidt},
    segmentation::{
        BuildDescriptor, CodeSegmentType, DataSegmentType, Descriptor, DescriptorBuilder,
        GateDescriptorBuilder, SegmentDescriptorBuilder, SegmentSelector, load_cs, load_ds,
        load_es, load_fs, load_gs, load_ss,
    },
    task::load_tr,
};

use crate::arch::x86_64::{irq_handler_user, irq_handler_entry};

#[repr(C, packed)]
pub(super) struct InterruptStackTable {
    pub reserved0: u32,
    pub rsp0: u64,
    pub rsp1: u64,
    pub rsp2: u64,
    pub reserved1: u64,
    pub ist1: u64,
    pub ist2: u64,
    pub ist3: u64,
    pub ist4: u64,
    pub ist5: u64,
    pub ist6: u64,
    pub ist7: u64,
    pub reserved2: u64,
    pub reserved3: u16,
    pub io_bp: u16,
}

#[repr(C, packed)]
pub(super) struct GlobalDescriptorTable {
    null: Descriptor,
    cs: Descriptor,
    ds: Descriptor,
    user_cs: Descriptor,
    user_ds: Descriptor,
    tss: Descriptor64,
}

#[repr(C, packed)]
pub(super) struct InterruptDescriptorTable {
    pub entries: [Descriptor64; 256],
}

impl GlobalDescriptorTable {
    pub const CS: u16 = 1;
    pub const DS: u16 = 2;
    pub const USER_CS: u16 = 3;
    pub const USER_DS: u16 = 4;
    pub const TSS: u16 = 5;

    pub fn new(ist: &InterruptStackTable) -> GlobalDescriptorTable {
        let cs: Descriptor =
            DescriptorBuilder::code_descriptor(0, 0xfffff, CodeSegmentType::ExecuteRead)
                .present()
                .dpl(Ring::Ring0)
                .limit_granularity_4kb()
                .l()
                .finish();

        let ds: Descriptor =
            DescriptorBuilder::data_descriptor(0, 0xfffff, DataSegmentType::ReadWrite)
                .present()
                .dpl(Ring::Ring0)
                .limit_granularity_4kb()
                .l()
                .finish();

        let tss: Descriptor64 = <DescriptorBuilder as GateDescriptorBuilder<u64>>::tss_descriptor(
            &raw const *ist as u64,
            size_of::<InterruptStackTable>() as u64,
            true,
        )
        .present()
        .finish();

        let user_cs: Descriptor =
            DescriptorBuilder::code_descriptor(0, 0xfffff, CodeSegmentType::ExecuteRead)
                .present()
                .dpl(Ring::Ring3)
                .limit_granularity_4kb()
                .l()
                .finish();

        let user_ds: Descriptor =
            DescriptorBuilder::data_descriptor(0, 0xfffff, DataSegmentType::ReadWrite)
                .present()
                .dpl(Ring::Ring3)
                .limit_granularity_4kb()
                .l()
                .finish();

        GlobalDescriptorTable {
            null: Descriptor::NULL,
            cs,
            ds,
            user_cs,
            user_ds,
            tss,
        }
    }

    pub unsafe fn load(&self) {
        unsafe {
            lgdt(&DescriptorTablePointer::new(self));
            load_cs(SegmentSelector::new(Self::CS, Ring::Ring0));
            load_ds(SegmentSelector::new(Self::DS, Ring::Ring0));
            load_ss(SegmentSelector::new(Self::DS, Ring::Ring0));
            load_es(SegmentSelector::new(Self::DS, Ring::Ring0));
            load_fs(SegmentSelector::new(Self::DS, Ring::Ring0));
            load_gs(SegmentSelector::new(Self::DS, Ring::Ring0));
            load_tr(SegmentSelector::new(Self::TSS, Ring::Ring0))
        }
    }
}

impl InterruptDescriptorTable {
    fn pack_idt_entry(addr: u64, ist: u8, dpl: Ring) -> Descriptor64 {
        DescriptorBuilder::interrupt_descriptor(
            SegmentSelector::new(GlobalDescriptorTable::CS, Ring::Ring0),
            addr,
        )
        .present()
        .dpl(dpl)
        .ist(ist)
        .finish()
    }

    pub fn new() -> InterruptDescriptorTable {
        let mut entries = [Descriptor64::default(); 256];

        let jmp_targets = {
            let mut entries = [0; 256];

            seq!(N in 0..=255 {
                // always switch to stack 1
                entries[N] = irq_handler_entry::<N> as *const () as u64;
            });

            entries[0x80] = irq_handler_user::<0x80> as *const () as u64;

            entries
        };

        for i in 0..=21 {
            entries[i] = Self::pack_idt_entry(jmp_targets[i], 1, Ring::Ring0);
        }

        for i in 32..=255 {
            entries[i] = Self::pack_idt_entry(jmp_targets[i], 1, Ring::Ring0);
        }

        entries[0x80] = Self::pack_idt_entry(jmp_targets[0x80], 1, Ring::Ring3);

        InterruptDescriptorTable { entries }
    }

    #[allow(dead_code)]
    pub unsafe fn load(&self) {
        unsafe {
            lidt(&DescriptorTablePointer::new(self));
        }
    }
}
