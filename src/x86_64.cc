/* Copyright (C) 2025 Ahmed Gheith and contributors.
 *
 * Use restricted to classroom projects.
 *
 * THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES
 * WITH REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF
 * MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR ANY
 * SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES
 * WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS, WHETHER IN AN ACTION
 * OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING OUT OF OR IN
 * CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.
 */

#include "x86_64.h"
#include "cpuid.h"
#include "machine.h"
#include "print.h"

//////////////
// Features //
//////////////

Features::Features() {

  uint32_t eax = 0;
  uint32_t ebx = 0;
  uint32_t ecx = 0;
  uint32_t edx = 0;

  __get_cpuid(0, &max_leaf, &id[0], &id[2], &id[1]);
  the_id[12] = 0;

  __get_cpuid(1, &eax, &ebx, &ecx, &edx);
  hasSSE3 = ecx & bit_SSE3;
  hasSSSE3 = ecx & bit_SSSE3;
  hasSSE4_1 = ecx & bit_SSE4_1;
  hasSSE4_2 = ecx & bit_SSE4_2;
  hasXSAVE = ecx & bit_XSAVE;
  hasOSXSAVE = ecx & bit_OSXSAVE;
  // hasMonitor = ecx & bit_MONITOR;
  // hasAVX = ecx & bit_AVX;
  onHypervisor = ecx & 0x80000000;

  __get_cpuid_count(7, 0, &eax, &ebx, &ecx, &edx);
  hasFSGSBASE = ebx & bit_FSGSBASE;
  // hasVMX = ecx & bit_VMX;

  __get_cpuid(0x80000001, &eax, &ebx, &ecx, &edx);
  hasSVM = ecx & 0x4;
  hasRDTSCP = ecx & (1 << 27);
}

void Features::dump() const {

  KPRINT("CPUID: ?\n", the_id);

  KPRINT("max leaf: ?\n", max_leaf);

  KPRINT("Features:\n");
  if (hasAVX) {
    KPRINT("|     hasAVX\n");
  } else if (hasSSE4_2) {
    KPRINT("|     hasSSE4_2\n");
  } else if (hasSSE4_1) {
    KPRINT("|     hasSSE4_1\n");
  } else if (hasSSSE3) {
    KPRINT("|     hasSSSE3\n");
  } else if (hasSSE3) {
    KPRINT("|     hasSSE3\n");
  }
  if (hasXSAVE) {
    KPRINT("|     hasXSAVE\n");
  }
  if (hasOSXSAVE) {
    KPRINT("|     hasOSXSAVE\n");
  }
  // if (hasMonitor) {
  //   KPRINT("|     hasMonitor\n");
  // }
  //  if (hasVMX) {
  //    KPRINT("|     hasVMX\n");
  //  }
  if (hasSVM) {
    KPRINT("|     hasSVM\n");
  }
  if (hasFSGSBASE) {
    KPRINT("|     hasFSGSBASE\n");
  }
  if (hasRDTSCP) {
    KPRINT("|     hasRDTSCP\n");
  }
}

/////////
// CR0 //
/////////

CR0::CR0() : ControlRegister<uint64_t>{get_cr0()} {}
const char *CR0::name() const { return "CR0"; }
const char *CR0::bit_name(int bit) const {
  switch (bit) {
  case 0:
    return "PE (Protection Enable)";
  case 1:
    return "MP (Monitor Coprocessor)";
  case 2:
    return "EM (Emulation)";
  case 3:
    return "TS (Task Switched)";
  case 4:
    return "ET (Extension Type)";
  case 5:
    return "NE (Numeric Error)";
  case 16:
    return "WP (Write Protect)";
  case 18:
    return "AM (Alignment Mask)";
  case 29:
    return "NW (Not Write-through)";
  case 30:
    return "CD (Cache Disable)";
  case 31:
    return "PG (Paging)";
  default:
    return "Unknown";
  }
}

/////////
// CR4 //
/////////

CR4::CR4() : ControlRegister<uint64_t>{get_cr4()} {}
const char *CR4::name() const { return "CR4"; }
const char *CR4::bit_name(int bit) const {
  switch (bit) {
  case 0:
    return "VME (Virtual-8086 Mode Extensions)";
  case 1:
    return "PVI (Protected-Mode Virtual Interrupts)";
  case 2:
    return "TSD (Time Stamp Disable)";
  case 3:
    return "DE (Debugging Extensions)";
  case 4:
    return "PSE (Page Size Extensions)";
  case 5:
    return "PAE (Physical Address Extension)";
  case 6:
    return "MCE (Machine-Check Enable)";
  case 7:
    return "PGE (Page Global Enable)";
  case 8:
    return "PCE (Performance-Monitoring Counter Enable)";
  case 9:
    return "OSFXSR (Operating System Support for FXSAVE and FXRSTOR "
           "instructions)";
  case 10:
    return "OSXMMEEXCPT (Operating System Support for Unmasked SIMD "
           "Floating-Point Exceptions)";
  case 13:
    return "VMXE (Virtual Machine Extensions Enable)";
  case 14:
    return "SMXE (Safer Mode Extensions Enable)";
  case 16:
    return "FSGSBase (Fast Global Base Switch)";
  case 17:
    return "PCIDE (PCID Enable)";
  default:
    return "Unknown";
  }
}

void CR4::setFSGSBASE() { set_cr4(get_cr4() | (1 << 16)); }
