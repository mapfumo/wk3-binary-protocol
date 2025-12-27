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

### Date: 2025-12-27 (continued)

#### TX State Machine Design (Implemented)

**Simplified Two-State Design**:
```rust
pub enum TxState {
    Idle,                    // Ready to send new packet
    WaitingForAck {          // Packet sent, waiting for ACK
        seq_num: u16,        // Which packet we're waiting for
        timeout_counter: u32, // Countdown in seconds until timeout
        retry_count: u8,     // How many retries attempted so far
    },
}
```

**Why Simplified?**
- Collapsed Sending/Retry/Success into state transitions within Idle/WaitingForAck
- Simpler RTIC resource management (tx_state is Shared resource)
- Clearer separation: Idle = can send new packet, WaitingForAck = ACK pending

**State Transitions**:
1. **Idle → WaitingForAck**: When packet is transmitted successfully
   - Stores seq_num, sets timeout_counter=2s, retry_count=0
2. **WaitingForAck → Idle**: When matching ACK received
   - Successful transmission complete
3. **WaitingForAck → WaitingForAck**: On NACK or timeout
   - Increments retry_count, resets timeout_counter=0 (immediate retry)
   - If retry_count >= MAX_RETRIES: transition to Idle (give up)

#### Implementation Details

**RTIC Resource Management**:
- `tx_state` is a **Shared resource** (not Local)
- Accessed by both `tim2_handler` (timeout countdown) and `uart4_handler` (ACK reception)
- Requires `.lock()` for safe concurrent access

**Timeout Handling**:
```rust
// In tim2_handler (1 Hz timer):
cx.shared.tx_state.lock(|state| {
    match *state {
        TxState::WaitingForAck { seq_num, timeout_counter, retry_count } => {
            if timeout_counter > 0 {
                // Countdown
                *state = TxState::WaitingForAck {
                    seq_num,
                    timeout_counter: timeout_counter - 1,
                    retry_count,
                };
            } else {
                // Timeout reached - will retry or give up
                if retry_count < MAX_RETRIES {
                    defmt::warn!("ACK timeout, retry {}/{}", retry_count + 1, MAX_RETRIES);
                } else {
                    defmt::error!("Max retries reached, giving up");
                    *state = TxState::Idle;
                }
            }
        }
        TxState::Idle => { /* Normal operation */ }
    }
});
```

**ACK Reception**:
```rust
// In uart4_handler:
cx.shared.tx_state.lock(|state| {
    if let TxState::WaitingForAck { seq_num, .. } = *state {
        if ack_pkt.seq_num == seq_num {
            defmt::info!("State: Idle (ACK matched, transmission successful)");
            *state = TxState::Idle;
        } else {
            defmt::warn!("ACK seq mismatch: expected {}, got {}", seq_num, ack_pkt.seq_num);
        }
    }
});
```

#### Retry Logic Parameters

**Configuration**:
- Max retries: 3
- ACK timeout: 2 seconds
- No backoff delay: Immediate retry on timeout/NACK (timeout_counter=0)

#### RX State Machine (Node 2)

**Simple Event-Driven Design** (not explicit state enum):
- Node 2 remains in implicit "listening" state
- On packet received:
  1. Validate CRC
  2. Send ACK (CRC valid) or NACK (CRC invalid)
  3. Return to listening
- No state machine needed - stateless receiver

#### Test Results (End-to-End)

**Milestone Achieved**: Full state machine working with ACK-based reliable delivery! 🎉

**Actual Log Extract** (First successful transmission with state machine):
```
Node 1 (Transmitter):
[INFO] Auto-transmit countdown reached 0
[INFO] Binary packet: 8 bytes data + 2 bytes CRC = 10 total, CRC: 0x9383
[INFO] Binary TX [AUTO]: 10 bytes sent, packet #1
[INFO] State: WaitingForAck (2s timeout)          ← State machine: Idle → WaitingForAck
[INFO] N1 UART: 5 bytes received
[INFO] N1 UART: 20 bytes received
[INFO] ACK received for packet #1
[INFO] State: Idle (ACK matched, transmission successful)  ← State machine: WaitingForAck → Idle
[INFO] Auto-transmit countdown reached 0
[INFO] Binary packet: 8 bytes data + 2 bytes CRC = 10 total, CRC: 0x31FC
[INFO] Binary TX [AUTO]: 10 bytes sent, packet #2
[INFO] State: WaitingForAck (2s timeout)
[INFO] N1 UART: 5 bytes received
[INFO] N1 UART: 20 bytes received
[INFO] ACK received for packet #2
[INFO] State: Idle (ACK matched, transmission successful)
```

Node 2 (Receiver):
```
[INFO] UART INT: 1 bytes, complete=true
[INFO] Processing buffer: 29 bytes
[INFO] CRC OK: 0x9383                              ← CRC validation passed
[INFO] Binary RX - T:27.3 H:58.87 G:355059 Pkt:1 RSSI:-27 SNR:13
[INFO] ACK sent for packet #1                      ← ACK transmitted back to Node 1
[INFO] N2 Timer: total_count=1, has_packet=true
```

**Complete State Machine Cycle Verified**:

1. Node 1 sends packet #1 (CRC: 0x9383)
2. Node 1 transitions: **Idle → WaitingForAck** (2s timeout armed)
3. Node 2 receives, validates CRC ✅
4. Node 2 sends ACK for packet #1
5. Node 1 receives ACK (20 bytes LoRa wrapper)
6. Node 1 transitions: **WaitingForAck → Idle** (transmission successful!)
7. Cycle repeats for packet #2, #3... continuously

**Success Metrics**:

- ✅ State transitions working correctly (Idle ↔ WaitingForAck)
- ✅ ACK reception triggers Idle transition
- ✅ Sequence number validation working
- ✅ Continuous operation over multiple packets (3+ tested)
- ✅ No timeouts or retries needed (perfect link quality at -27 dBm RSSI, SNR:13)
- ✅ Average round-trip time: <1 second (well within 2s timeout)

**What This Proves**:

- RTIC Shared resource management working (tx_state locked properly)
- No race conditions between tim2_handler and uart4_handler
- Sequence number matching prevents spurious ACKs
- System ready for timeout/retry testing

**Next Steps**:

- ✅ Test timeout behavior (COMPLETED - see below)
- ✅ Test max retry limit (COMPLETED - verified "giving up" after 3 retries)
- ⏭️ Test NACK handling (inject CRC errors on Node 2)
- ⏭️ Performance analysis and final documentation

---

#### Timeout/Retry Testing Results

**Test Setup**: Changed Node 2 to different LoRa network ID (99 vs 18) to prevent communication

**Actual Log from Node 1** (No ACKs received):

```
[INFO] Binary TX [AUTO]: 10 bytes sent, packet #1
[INFO] State: WaitingForAck (2s timeout)
[INFO] N1 UART: 5 bytes received                           ← LoRa "+OK" response, not an ACK
[WARN] ACK timeout for packet #1, attempt 2/3, will keep waiting
[WARN] ACK timeout for packet #1, attempt 3/3, will keep waiting
[ERROR] Max retries (3) exceeded for packet #1, giving up  ← State machine gives up!
[INFO] Auto-transmit countdown reached 0
[INFO] Binary TX [AUTO]: 10 bytes sent, packet #2          ← Continues with next packet
[INFO] State: WaitingForAck (2s timeout)
[INFO] N1 UART: 5 bytes received
[WARN] ACK timeout for packet #2, attempt 2/3, will keep waiting
```

**Timeout Behavior Verified**:

- ✅ Each timeout period = 2 seconds (configurable via `ACK_TIMEOUT_SECS`)
- ✅ Retry count increments correctly (0 → 1 → 2 → 3)
- ✅ After 3 attempts (total ~6 seconds), state transitions to Idle
- ✅ System continues operating (sends packet #2, #3, etc.)
- ✅ No crashes or hangs - graceful degradation

**Recovery Testing**: Restored Node 2 to network 18

```log
[INFO] Binary TX [AUTO]: 10 bytes sent, packet #6
[INFO] State: WaitingForAck (2s timeout)
[INFO] N1 UART: 5 bytes received
[INFO] N1 UART: 20 bytes received                          ← ACK received!
[INFO] ACK received for packet #6
[INFO] State: Idle (ACK matched, transmission successful)  ← Recovery successful!
```

**Recovery Verified**:

- ✅ System immediately resumes normal operation when Node 2 returns
- ✅ No state corruption or stuck conditions
- ✅ Packet sequence numbers continue correctly (#6, #7, #8...)

**Key Findings**:

1. **Timeout Implementation Works Correctly**: The 1Hz timer counts down `timeout_counter` from 2→1→0, then increments `retry_count`
2. **Max Retry Limit Enforced**: After 3 attempts, ERROR log appears and state → Idle
3. **Graceful Degradation**: System doesn't block or crash on communication failure
4. **Fast Recovery**: When link restored, next transmission immediately succeeds
5. **Total Timeout Duration**: ~6 seconds (3 attempts × 2s) before giving up

**What We Learned**:

- **Design Limitation**: Current implementation doesn't actually retransmit the same packet - it just waits longer. True packet retransmission would require storing the serialized packet.
- **Pragmatic Choice**: For sensor data that changes every 10 seconds, "giving up and sending fresh data" is acceptable behavior.
- **Future Enhancement**: Could add packet buffer to retransmit exact same data if needed for critical telemetry.

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

## Range Testing Results

### Test Date: 2025-12-27

#### Test Environment
- **Location**: Suburban residential area
- **Weather**: Light clouds, 15°C
- **Conditions**: No line of sight for most distances, buildings and trees in path
- **Terrain**: Downhill from 100m onwards

#### Measured Results

| Distance | RSSI (dBm) | SNR (dB) | Packet Loss (%) | Environment Notes |
|----------|------------|----------|-----------------|-------------------|
| 15m | -45 | 13 | 0% | End of house (kitchen), wall obstacles, no LoS |
| 30m | -62 | 13 | 1% | Verandah, wall obstacles |
| 60m | -72 | 12 | 1% | Near driveway, wall obstacles |
| 100m | -82 | 11 | 1% | Street junction, trees in street, downhill, no LoS |
| 150m | -91 | 4 | 2% | Next street, trees & tall buildings, no LoS |
| 400m | -100 | -2 | 2% | Another street, trees & tall buildings, no LoS |
| 600m | -107 | -6 | 5% | Outside local church, no LoS |

#### Key Observations

**Signal Strength Degradation**:
- RSSI degrades approximately 10 dBm per doubling of distance (textbook behavior)
- At 600m: -107 dBm is near LoRa sensitivity limit (-110 to -120 dBm typical)
- Signal propagation follows expected path loss model despite obstacles

**SNR Performance**:
- SNR remains healthy (>10 dB) up to 100m distance
- SNR drops below 5 dB at 150m+ (noise floor increasing relative to signal)
- **LoRa spread spectrum advantage**: Still functional with negative SNR (-2 dB at 400m, -6 dB at 600m)
- Demonstrates LoRa's robustness in low SNR conditions

**Packet Loss Analysis**:
- 0-100m: Essentially perfect (0-1% loss = >99% success rate)
- 150-400m: Excellent (2% loss = 98% success rate)
- 600m: Very usable (5% loss = 95% success rate)
- Earlier observed 70-80% success rate likely due to indoor RF interference, not range

**Real-World Performance**:
- ✅ **600m range achieved** through suburban obstacles (no line of sight)
- ✅ **95% packet success rate at maximum tested distance**
- ✅ **>98% success rate up to 400m** with negative SNR
- ✅ **Predictable signal degradation** enables range estimation

#### Technical Analysis

**Why LoRa Works with Negative SNR**:
- Chirp spread spectrum spreads signal across wide bandwidth
- Processing gain recovers signal below noise floor
- Forward error correction adds redundancy
- Typical LoRa can achieve -7.5 to -20 dB SNR sensitivity depending on spreading factor

**Estimated Maximum Range** (extrapolating from data):
- Line of sight: Could potentially reach 1-2 km with same settings
- Urban/suburban (obstructed): 600-800m practical limit
- Sensitivity limit: ~-120 dBm → suggests 800-1000m maximum with obstacles

**Configuration Used**:
- Module: RYLR998 (868/915 MHz)
- Spreading Factor: Default (likely SF7 or SF9)
- Bandwidth: Default (likely 125 kHz)
- Coding Rate: Default (likely 4/5)
- Power: Default (likely +20 dBm)

#### Portfolio Impact

This data demonstrates:
1. **Real-world validation** of LoRa performance in challenging environments
2. **Predictable RF behavior** (path loss follows theory)
3. **Robust communication** even with negative SNR
4. **Production-ready reliability** (>95% success at 600m)

**Comparable to industrial deployments** - many commercial LoRa sensors operate at similar success rates over comparable distances in urban environments.

---

_Week 3 Notes - Complete_
_Part of 12-Week IIoT Systems Engineer Transition Plan_
