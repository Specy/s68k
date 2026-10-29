// A C interface to Moira (MIT, https://github.com/dirkwhoffmann/Moira) as a
// 68010, for the differential harness of s68k's 68010 dialect
// (src/conformance/moira.rs). build.rs compiles it with Moira's core, whose
// configuration it sets to precise timing, without Musashi's mimicry and with
// address errors, so that Moira counts the cycles of its accesses as a 68010
// does. Moira keeps one processor here, so one test at a time may use it.

#include <cstdint>
#include <cstring>
#include <vector>

#include "Moira.h"

using namespace moira;

namespace {

uint8_t memory[0x1000000];
std::vector<uint32_t> written;

const uint32_t mask = 0x00FFFFFF;

class Processor : public Moira {
    u8 read8(u32 address) const override { return memory[address & mask]; }

    u16 read16(u32 address) const override {
        return u16(memory[address & mask] << 8 | memory[(address + 1) & mask]);
    }

    u32 read32(u32 address) const override {
        return u32(read16(address)) << 16 | read16(address + 2);
    }

    void write8(u32 address, u8 value) const override {
        memory[address & mask] = value;
        written.push_back(address & mask);
    }

    void write16(u32 address, u16 value) const override {
        write8(address, u8(value >> 8));
        write8(address + 1, u8(value));
    }

    void write32(u32 address, u32 value) const override {
        write16(address, u16(value >> 16));
        write16(address + 2, u16(value));
    }
};

Processor *processor = nullptr;

void put_long(uint32_t address, uint32_t value) {
    for (int byte = 0; byte < 4; byte++) {
        memory[(address + byte) & mask] = uint8_t(value >> (24 - 8 * byte));
    }
}

}  // namespace

extern "C" {

// The 16 MB the processor reads and writes, which the harness fills.
uint8_t *s68k_moira_memory() { return memory; }

// Makes the processor, a 68010, once.
void s68k_moira_init() {
    if (processor == nullptr) {
        processor = new Processor();
        processor->setModel(Model::M68010);
    }
}

// Resets the processor into the state given, the instruction at `pc` in its
// prefetch queue: the reset reads the stack pointer and the program counter
// from $0 and $4, which hold them while it does and get their bytes back
// after, then every register is set.
void s68k_moira_load(const uint32_t *d, const uint32_t *a, uint32_t usp, uint32_t ssp,
                     uint16_t sr, uint32_t pc, uint32_t vbr, uint8_t sfc, uint8_t dfc) {
    uint8_t vectors[8];
    std::memcpy(vectors, memory, 8);
    put_long(0, ssp);
    put_long(4, pc);
    processor->reset();
    std::memcpy(memory, vectors, 8);
    processor->setPC0(pc);
    processor->setSR(sr);
    processor->setUSP(usp);
    processor->setISP(ssp);
    for (int n = 0; n < 8; n++) processor->setD(n, d[n]);
    for (int n = 0; n < 7; n++) processor->setA(n, a[n]);
    processor->setVBR(vbr);
    processor->setSFC(sfc);
    processor->setDFC(dfc);
    processor->setClock(0);
    written.clear();
}

// Runs one instruction, and answers the cycles it took, or -1 when an
// exception of Moira's escaped it: an address error inside loop mode, which
// Moira's loop dispatch does not catch.
int64_t s68k_moira_step() {
    int64_t before = processor->getClock();
    try {
        processor->execute();
    } catch (...) {
        return -1;
    }
    return processor->getClock() - before;
}

// The registers as they are now.
void s68k_moira_state(uint32_t *d, uint32_t *a, uint32_t *usp, uint32_t *ssp, uint16_t *sr,
                      uint32_t *pc, uint32_t *vbr, uint8_t *sfc, uint8_t *dfc) {
    for (int n = 0; n < 8; n++) d[n] = processor->getD(n);
    for (int n = 0; n < 7; n++) a[n] = processor->getA(n);
    *usp = processor->getUSP();
    *ssp = processor->getISP();
    *sr = processor->getSR();
    *pc = processor->getPC();
    *vbr = processor->getVBR();
    *sfc = uint8_t(processor->getSFC());
    *dfc = uint8_t(processor->getDFC());
}

// Every byte written since the last load, by address.
size_t s68k_moira_written(const uint32_t **addresses) {
    *addresses = written.data();
    return written.size();
}

}  // extern "C"
