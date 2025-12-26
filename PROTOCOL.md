# Binary Protocol Specification - Week 3

## Overview

This document defines the binary protocol used for LoRa communication between Node 1 (sensor transmitter) and Node 2 (gateway receiver) in Week 3 of the 12-week IIoT program.

**Protocol Goals**:
- Reduce payload size compared to text protocol (Week 2)
- Add integrity checking with CRC-16
- Enable reliable delivery with ACK/NACK mechanism
- Support future protocol evolution

---

## Message Types

### 1. SensorData (0x01)

Sent by Node 1 to transmit sensor readings.

**Structure**:
```rust
#[derive(Serialize, Deserialize, Debug)]
pub struct SensorDataPacket {
    pub seq_num: u16,          // Sequence number (0-65535, wraps)
    pub temperature: i16,      // Centidegrees (e.g., 2710 = 27.1°C)
    pub humidity: u16,         // Basis points (e.g., 5600 = 56.0%)
    pub gas_resistance: u32,   // Gas resistance in ohms
    pub crc: u16,              // CRC-16 of all fields above
}
```

**Size**: ~14 bytes (postcard serialized)

**Field Details**:
- `seq_num`: Increments with each transmission, used for duplicate detection
- `temperature`: Signed integer, range -327.68°C to +327.67°C
- `humidity`: Unsigned integer, range 0.00% to 655.35%
- `gas_resistance`: Unsigned 32-bit, sufficient for BME680 range (0-400kΩ typical)
- `crc`: CRC-16-IBM-SDLC calculated over all preceding fields

### 2. Ack (0x02)

Sent by Node 2 to confirm successful reception and validation.

**Structure**:
```rust
#[derive(Serialize, Deserialize, Debug)]
pub struct AckPacket {
    pub ack_seq_num: u16,      // Sequence number being acknowledged
    pub crc: u16,              // CRC-16 of ack_seq_num
}
```

**Size**: ~4 bytes (postcard serialized)

### 3. Nack (0x03)

Sent by Node 2 when CRC validation fails.

**Structure**:
```rust
#[derive(Serialize, Deserialize, Debug)]
pub struct NackPacket {
    pub nack_seq_num: u16,     // Sequence number being rejected
    pub error_code: u8,        // Error reason (0x01 = CRC fail)
    pub crc: u16,              // CRC-16 of above fields
}
```

**Size**: ~5 bytes (postcard serialized)

---

## Packet Format

### Over-the-Air Format

Every packet transmitted via LoRa follows this structure:

```
┌────────────┬──────────────┬─────────────┬──────────┐
│ Length (1) │ Type (1)     │ Payload (N) │ CRC (2)  │
└────────────┴──────────────┴─────────────┴──────────┘
```

**Fields**:
- **Length** (1 byte): Total packet length excluding this byte (range: 1-255)
- **Type** (1 byte): Message type identifier
  - `0x01` = SensorData
  - `0x02` = Ack
  - `0x03` = Nack
- **Payload** (N bytes): Postcard-serialized message struct
- **CRC** (2 bytes): CRC-16-IBM-SDLC of entire packet (Length + Type + Payload)

### AT Command Encapsulation

The binary packet is transmitted via RYLR998 AT command:

```
AT+SEND=<Address>,<Length>,<Binary Data>\r\n
```

**Example** (SensorData):
```
AT+SEND=2,16,<14 bytes of binary data>\r\n
```

**Note**: Binary data may contain non-printable bytes - RYLR998 handles this transparently.

---

## CRC Calculation

### Algorithm: CRC-16-IBM-SDLC

**Polynomial**: 0x1021 (x^16 + x^12 + x^5 + 1)
**Initial Value**: 0xFFFF
**XOR Out**: 0xFFFF

**Rationale**: Industry standard, good error detection, available in `crc` crate.

### Implementation

```rust
use crc::{Crc, CRC_16_IBM_SDLC};

pub const CRC16: Crc<u16> = Crc::<u16>::new(&CRC_16_IBM_SDLC);

pub fn calculate_crc(data: &[u8]) -> u16 {
    CRC16.checksum(data)
}
```

### CRC Coverage

**SensorDataPacket**:
- CRC covers: `seq_num` + `temperature` + `humidity` + `gas_resistance`
- CRC does NOT cover itself (calculated first, appended last)

**Over-the-Air Packet**:
- Additional CRC covers: `Length` + `Type` + `Payload`
- Provides two layers of integrity checking

---

## State Machines

### Node 1 (Transmitter) State Machine

```
┌──────┐  Timer       ┌─────────┐  TX Done     ┌─────────────┐
│ Idle │─────────────>│ Sending │─────────────>│ WaitingAck  │
└──────┘              └─────────┘              └─────────────┘
   ^                                                  │
   │                                                  │ ACK Received
   │                                            ┌─────┴─────┐
   │                                            │  Success  │
   │<───────────────────────────────────────────┴───────────┘
   │                                                  │
   │                                                  │ Timeout/NACK
   │                                            ┌─────┴─────┐
   │                                            │   Retry   │
   │<───────────────────────────────────────────┴───────────┘
        (if max_retries exceeded)
```

**States**:
1. **Idle**: Waiting for next transmission cycle
2. **Sending**: Serializing and transmitting packet
3. **WaitingAck**: Listening for ACK/NACK with timeout
4. **Success**: ACK received, log success
5. **Retry**: Increment retry counter, re-send or give up

**Parameters**:
- **Transmission Interval**: 10 seconds
- **ACK Timeout**: 500ms
- **Max Retries**: 3
- **Retry Backoff**: None (constant timeout)

### Node 2 (Receiver) State Machine

```
┌───────────┐  Packet Arrives   ┌────────────┐  CRC Valid    ┌────────────┐
│ Listening │──────────────────>│ Validating │──────────────>│ SendingAck │
└───────────┘                   └────────────┘               └────────────┘
     ^                                 │                            │
     │                                 │ CRC Invalid                │
     │                           ┌─────┴──────┐                     │
     │                           │ SendingNack│                     │
     │                           └────────────┘                     │
     │                                 │                            │
     │<────────────────────────────────┴────────────────────────────┘
              ACK/NACK Sent
```

**States**:
1. **Listening**: UART interrupt listening for packets
2. **Validating**: CRC check on received packet
3. **SendingAck**: Transmit ACK if valid
4. **SendingNack**: Transmit NACK if invalid

---

## Sequence Number Management

### Purpose

- Detect duplicate packets (retries after ACK loss)
- Track packet ordering
- Debug transmission statistics

### Implementation

**Node 1**:
```rust
static mut SEQUENCE_NUM: u16 = 0;

fn get_next_seq_num() -> u16 {
    unsafe {
        SEQUENCE_NUM = SEQUENCE_NUM.wrapping_add(1);
        SEQUENCE_NUM
    }
}
```

**Node 2**:
```rust
static mut LAST_SEQ_NUM: Option<u16> = None;

fn is_duplicate(seq_num: u16) -> bool {
    unsafe {
        if let Some(last) = LAST_SEQ_NUM {
            if seq_num == last {
                return true;
            }
        }
        LAST_SEQ_NUM = Some(seq_num);
        false
    }
}
```

### Wraparound Handling

Sequence numbers are `u16`, wrapping at 65536. This is acceptable for:
- Short-term duplicate detection (< 65536 packets between duplicates)
- This project's transmission rate (1 packet/10s = 7.5 days to wrap)

---

## Comparison: Text vs Binary Protocol

### Payload Size

| Metric | Text (Week 2) | Binary (Week 3) | Reduction |
|--------|---------------|-----------------|-----------|
| Sensor Data | 24 bytes | ~14 bytes | 42% |
| AT Command Overhead | ~15 bytes | ~15 bytes | 0% |
| Total Packet | ~39 bytes | ~29 bytes | 26% |
| CRC Overhead | 0 bytes | 2 bytes | N/A |
| ACK Packet | N/A | ~4 bytes | N/A |

**Winner**: Binary is 26% smaller overall, despite adding CRC.

### Parsing Complexity

**Text**:
- String parsing with `core::str::from_utf8()`
- Float/integer conversion with string slicing
- Error-prone (malformed strings)

**Binary**:
- Direct deserialization with `postcard::from_bytes()`
- Type-safe structs
- Compile-time guarantees

**Winner**: Binary is simpler and safer.

### Extensibility

**Text**:
- Adding fields requires string format changes
- Backward compatibility difficult
- No versioning built-in

**Binary**:
- Serde supports optional fields
- Postcard is self-describing
- Version field can be added easily

**Winner**: Binary is more extensible.

---

## Error Handling

### Node 1 (TX)

**Serialization Failure**:
```rust
match postcard::to_slice(&sensor_data, &mut buffer) {
    Ok(bytes) => transmit(bytes),
    Err(e) => {
        defmt::error!("Serialization failed: {:?}", e);
        // Skip this transmission, try again next cycle
    }
}
```

**Retry Exhaustion**:
```rust
if retry_count >= MAX_RETRIES {
    defmt::warn!("Packet {} failed after {} retries", seq_num, MAX_RETRIES);
    // Log failure, increment failure counter, return to Idle
}
```

### Node 2 (RX)

**CRC Failure**:
```rust
if calculated_crc != received_crc {
    defmt::warn!("CRC mismatch: calc={}, recv={}", calculated_crc, received_crc);
    send_nack(seq_num, ERROR_CRC_FAIL);
    return;
}
```

**Deserialization Failure**:
```rust
match postcard::from_bytes::<SensorDataPacket>(&buffer) {
    Ok(packet) => process(packet),
    Err(e) => {
        defmt::error!("Deserialization failed: {:?}", e);
        // Discard packet, continue listening
    }
}
```

---

## Testing Strategy

### Unit Tests (Desktop)

Test serialization/deserialization with `std`:
```bash
cargo test --lib
```

### Integration Tests (Hardware)

1. **Happy Path**: Node 1 sends, Node 2 ACKs, no retries
2. **CRC Failure**: Inject bit flip, verify NACK sent
3. **ACK Loss**: Suppress ACK, verify retry behavior
4. **Duplicate Detection**: Send duplicate seq_num, verify ignored
5. **Sequence Wraparound**: Test at seq_num = 65535 → 0

### Performance Tests

- Measure serialization time (µs)
- Measure round-trip latency (ms)
- Calculate packet delivery rate (%)
- Compare with Week 2 text protocol

---

## Future Enhancements (Week 4+)

- **Multi-Sensor Support**: Add node_id to differentiate sources
- **Batching**: Combine multiple readings in one packet
- **Compression**: LZ4/DEFLATE for gas resistance values
- **Adaptive Retry**: Exponential backoff based on RSSI/SNR
- **Downlink Commands**: Node 2 → Node 1 configuration updates

---

## References

- **Postcard Format**: https://docs.rs/postcard/
- **CRC-16 Spec**: https://en.wikipedia.org/wiki/Cyclic_redundancy_check
- **Serde**: https://serde.rs/
- **RYLR998 AT Commands**: Datasheet Section 4

---

_Last Updated_: [Current Date]
_Version_: 1.0
_Status_: Specification complete, implementation in progress
