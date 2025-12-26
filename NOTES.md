# Week 3 Learning Notes - Binary Protocol & State Machine

## Overview

This document tracks technical insights, design decisions, and key learnings while implementing binary serialization and reliable transmission for the LoRa sensor network.

---

## Day 1: Binary Protocol Design

### Date: 2025-12-27

#### Understanding: Text vs Binary - What's Actually Different?

**The Fundamental Question**: "We're sending data between nodes in binary format - how does that differ from text when it's all bytes at the end?"

**everything is bytes at the end of the day**. Here's the key difference:

**Text Protocol (Week 2)**:

```rust
// We create a string: "T:26.3H:58.6G:86967#0002"
let mut payload: String<128> = String::new();
core::write!(payload, "T:{:.1}H:{:.1}G:{:.0}#{:04}", temp_c, humid_pct, gas, counter);
```

Bytes sent: `54 3A 32 36 2E 33 48 3A 35 38 2E 36 47 3A 38 36 39 36 37 23 30 30 30 32`

- That's ASCII characters: `T`, `:`, `2`, `6`, `.`, `3`, `H`, `:`, etc.
- **24-25 bytes** for human-readable text
- Each number converted to ASCII digits (wasteful!):
  - Temperature `26.3` = 4 ASCII chars = 4 bytes
  - Humidity `58.6` = 4 ASCII chars = 4 bytes
  - Gas `86967` = 5 ASCII chars = 5 bytes

**Binary Protocol (Week 3)**:

```rust
// We pack the actual numeric values directly
let binary_packet = SensorDataPacket {
    seq_num: 2,                    // u16 = 2 bytes
    temperature: 263,              // i16 (26.3°C * 10) = 2 bytes
    humidity: 5860,                // u16 (58.6% * 100) = 2 bytes
    gas_resistance: 86967,         // u32 = 4 bytes
};
// Postcard serializes this efficiently
```

Bytes sent (after postcard): `02 00 07 01 E4 16 E7 53 01 00`

- **8-10 bytes** of compact binary data
- Each number uses minimum representation:
  - `seq_num: 2` → `02 00` (2 bytes)
  - `temperature: 263` → `07 01` (2 bytes vs 4 ASCII chars "26.3")
  - `humidity: 5860` → `E4 16` (2 bytes vs 4 ASCII chars)
  - `gas_resistance: 86967` → `E7 53 01 00` (4 bytes vs 5 ASCII chars)

**Key Differences**:

1. **Efficiency**

   - Text: Number 86967 = `"86967"` = 5 bytes (one per digit)
   - Binary: Number 86967 = `0x00015367` = 4 bytes (direct value)

2. **Parsing**

   - Text: Must parse strings, find delimiters, convert ASCII to numbers
   - Binary: Deserialize directly to struct with postcard

3. **Type Safety**

   - Text: Can send invalid data like `"T:ABC"` - receiver crashes
   - Binary: Postcard validates format - can't send string where number expected

4. **Over-the-Air**
   - Both use RYLR998 AT command: `AT+SEND=2,<length>,<payload>`
   - LoRa module doesn't care if payload is ASCII text or raw binary - just transmits bytes!

**Measured Results**:

- Text protocol: 24-25 bytes
- Binary protocol: 8 bytes data + 2 bytes CRC = **10 bytes total**
- **Size reduction: 60%** (10 vs 25 bytes, exceeds 40% target!)

**LoRa Capacity Context**:

- RYLR998 payload capacity: 240 bytes maximum
- Text protocol utilization: 24 bytes / 240 bytes = **10%**
- Binary protocol utilization: 10 bytes / 240 bytes = **4.2%**
- **Headroom for expansion**: Could fit 24 sensor readings in single message
- **Benefits**:
  - Lower airtime (reduced duty cycle impact)
  - Lower power consumption (less TX time)
  - Room for future features (ACK packets, timestamps, multi-sensor data)

**CRC-16 Validation (Implemented 2025-12-27)**:

- **Algorithm**: CRC-16-IBM-3740 (CCITT with 0xFFFF initial value)
- **Overhead**: 2 bytes (20% of data payload)
- **Status**: ✅ Working end-to-end
- **Test Results**:
  - Node 1 calculates and appends CRC to payload
  - Node 2 validates CRC before deserialization
  - Example: Packet #2 → CRC: 0x22FE (matched on both sides)
  - Signal quality: RSSI: -20 dBm, SNR: 13 dB

#### Design Decisions

**Why Binary over Text?**

- Payload size reduction (estimated 40% smaller)
- Built-in type safety with Serde
- CRC validation for integrity
- Faster parsing (no string conversion)
- Professional protocol design practice

**CRC-16 Selection**

- Industry standard for embedded systems
- Good error detection for typical LoRa noise
- Small overhead (2 bytes)
- Hardware acceleration available on many MCUs

**Message Type Design**

```rust
enum MessageType {
    SensorData,  // Node 1 → Node 2
    Ack,         // Node 2 → Node 1 (success)
    Nack,        // Node 2 → Node 1 (CRC fail)
}
```

#### Packet Structure Considerations

**Tradeoffs**:

- Length prefix vs delimiter: Chose length prefix for binary safety
- Fixed vs variable length: Variable with length prefix for flexibility
- CRC placement: End of packet (standard practice)

---

## Day 2: Postcard Integration

### Date: [To be filled]

#### Postcard Selection Rationale

**Why Postcard?**

- `no_std` compatible (essential for embedded)
- Smaller serialized output than bincode
- Self-describing format (versioning friendly)
- Active maintenance and embedded Rust community adoption

**Alternatives Considered**:

- `bincode`: Larger output, less embedded-focused
- Custom encoding: More work, error-prone
- `rmp-serde` (MessagePack): Good, but postcard is lighter

#### Integration Challenges

[To be documented during implementation]

#### Performance Measurements

**Serialized Sizes** (measured):

- Text format: 24 bytes (Week 2)
- Binary format: [TBD] bytes
- Size reduction: [TBD]%

---

## Day 3: State Machine Implementation

### Date: [To be filled]

#### TX State Machine Design

```
States:
- Idle: Waiting for timer to trigger transmission
- Sending: Transmitting packet via LoRa
- WaitingForAck: Listening for ACK with timeout
- Retry: Re-sending after timeout/NACK
- Success: ACK received, return to Idle

Transitions:
- Idle → Sending (timer fires)
- Sending → WaitingForAck (packet sent)
- WaitingForAck → Success (ACK received)
- WaitingForAck → Retry (timeout or NACK)
- Retry → Sending (attempt < max_retries)
- Retry → Idle (max_retries exceeded)
```

#### RX State Machine Design

```
States:
- Listening: Waiting for packet
- Validating: CRC check in progress
- SendingAck: Transmitting ACK
- SendingNack: Transmitting NACK

Transitions:
- Listening → Validating (packet received)
- Validating → SendingAck (CRC valid)
- Validating → SendingNack (CRC invalid)
- SendingAck/Nack → Listening (response sent)
```

#### Retry Logic Parameters

**Configuration**:

- Max retries: 3
- Initial timeout: 500ms
- Backoff strategy: [Linear/Exponential - TBD]

---

## Day 4: Integration & Testing

### Date: [To be filled]

#### Integration Steps

[To be documented]

#### Test Results

**End-to-End Test**:

- Packets sent: [TBD]
- ACKs received: [TBD]
- Retries needed: [TBD]
- Final success rate: [TBD]%

---

## Day 5: Performance Analysis

### Date: [To be filled]

#### Text vs Binary Comparison

| Metric            | Text (Week 2) | Binary (Week 3) | Improvement |
| ----------------- | ------------- | --------------- | ----------- |
| Payload Size      | 24-25 bytes   | 10 bytes        | **60%**     |
| LoRa Utilization  | 10%           | 4.2%            | 58% less    |
| Max Readings/Msg  | ~9 readings   | ~24 readings    | 2.7x more   |
| Type Safety       | No            | Yes (serde)     | ✓           |
| CRC Validation    | No            | Yes (CRC-16)    | ✅ Working  |
| Parsing Time      | [TBD] µs      | [TBD] µs        | [TBD]%      |
| Reliability       | 100%          | [TBD]%          | N/A         |

#### Round-Trip Latency

**Measured Values**:

- Node 1 sends → Node 2 receives: [TBD] ms
- Node 2 sends ACK → Node 1 receives: [TBD] ms
- Total RTT: [TBD] ms

---

## Day 6: Optimization & Edge Cases

### Date: [To be filled]

#### Edge Cases Handled

- Duplicate packets (same sequence number)
- Out-of-order delivery
- CRC collisions (rare but possible)
- ACK lost (timeout triggers retry)
- Buffer overflow on UART

#### Optimizations Applied

[To be documented]

---

## Day 7: Documentation & Review

### Date: [To be filled]

#### Week 3 Key Learnings

[To be documented at week end]

#### Challenges Overcome

[To be documented]

#### Skills Developed

- Serde in `no_std` environment
- Binary protocol design
- State machine implementation in RTIC
- CRC integrity checking
- Timeout handling with hardware timers

---

## Technical Deep Dives

### Serde + no_std

**Key Points**:

- Must use `default-features = false`
- Need `derive` feature for `#[derive(Serialize, Deserialize)]`
- Cannot use `std::` types (String, Vec, etc.) - use `heapless` instead

**Example**:

```rust
#[derive(Serialize, Deserialize, Debug)]
pub struct SensorData {
    pub seq_num: u16,
    pub temp: i16,
    // ...
}
```

### CRC-16 Implementation

**Algorithm**: [CRC-16-CCITT / CRC-16-ANSI - TBD]

**Integration**:

```rust
use crc::{Crc, CRC_16_IBM_SDLC};

pub const CRC16: Crc<u16> = Crc::<u16>::new(&CRC_16_IBM_SDLC);

fn calculate_crc(data: &[u8]) -> u16 {
    CRC16.checksum(data)
}
```

### Postcard Serialization

**Usage Pattern**:

```rust
// Serialize
let mut buffer = [0u8; 32];
let serialized = postcard::to_slice(&sensor_data, &mut buffer)?;

// Deserialize
let packet: SensorDataPacket = postcard::from_bytes(&buffer)?;
```

---

## RTIC Patterns Used

### State Machine as Shared Resource

```rust
#[shared]
struct Shared {
    tx_state: TxState,
    rx_state: RxState,
    // ...
}
```

### Timeout Handling with Timer

```rust
#[task(binds = TIM2, shared = [tx_state], local = [timeout_counter])]
fn tim2_handler(mut cx: tim2_handler::Context) {
    cx.shared.tx_state.lock(|state| {
        match state {
            TxState::WaitingForAck => {
                // Check timeout, transition to Retry
            }
            _ => {}
        }
    });
}
```

---

## Debugging Notes

### Common Issues

[To be documented as encountered]

### Logic Analyzer Captures

[To be added if needed]

---

## References

- [Postcard Docs](https://docs.rs/postcard/)
- [Serde no_std](https://serde.rs/no-std.html)
- [CRC Theory](https://en.wikipedia.org/wiki/Cyclic_redundancy_check)
- [RTIC Book - Software Tasks](https://rtic.rs/dev/book/en/)

---

_Week 3 Notes - In Progress_
_Part of 12-Week IIoT Systems Engineer Transition Plan_
