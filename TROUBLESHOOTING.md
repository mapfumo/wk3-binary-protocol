# Troubleshooting Guide - Week 3: Binary Protocol

This document tracks issues encountered during Week 3 implementation and their solutions.

---

## Table of Contents

1. [Postcard Serialization Issues](#1-postcard-serialization-issues)
2. [CRC Validation Failures](#2-crc-validation-failures)
3. [State Machine Timing](#3-state-machine-timing)
4. [ACK/NACK Handling](#4-acknack-handling)
5. [Buffer Management](#5-buffer-management)
6. [Sequence Number Wraparound](#6-sequence-number-wraparound)

---

## 1. Postcard Serialization Issues

### Problem: [To be documented]

**Symptoms**:
[TBD]

**Root Cause**:
[TBD]

**Solution**:
[TBD]

**References**:
- Postcard docs: https://docs.rs/postcard/

---

## 2. CRC Validation Failures

### Problem: [To be documented]

**Symptoms**:
[TBD]

**Root Cause**:
[TBD]

**Solution**:
[TBD]

**Prevention**:
- Always calculate CRC before transmission
- Validate immediately on reception
- Log both calculated and received CRC for debugging

---

## 3. State Machine Timing

### Problem: [To be documented]

**Symptoms**:
[TBD]

**Root Cause**:
[TBD]

**Solution**:
[TBD]

**Lessons Learned from Week 2**:
- Keep interrupt handlers simple and fast
- Separate fast operations (UART) from slow operations (I2C display)
- Process state transitions outside interrupt context when possible

---

## 4. ACK/NACK Handling

### Problem: [To be documented]

**Symptoms**:
[TBD]

**Root Cause**:
[TBD]

**Solution**:
[TBD]

---

## 5. Buffer Management

### Problem: [To be documented]

**Symptoms**:
[TBD]

**Root Cause**:
[TBD]

**Solution**:
[TBD]

**Best Practices from Week 2**:
- Size buffers for maximum expected payload + overhead
- RYLR998 supports 240-byte payloads (not LoRaWAN's 51-byte limit)
- Use `heapless::String<N>` with generous N values
- Check return values from `core::write!()`

---

## 6. Sequence Number Wraparound

### Problem: [To be documented]

**Symptoms**:
[TBD]

**Root Cause**:
[TBD]

**Solution**:
[TBD]

---

## General Debugging Tips

### From Week 2 Experience

1. **Trust Working Baselines**: If previous version worked, hardware is likely fine
2. **Compare with Last Known Good**: Use git diff to identify changes
3. **Simplicity First**: Remove complex code before adding more
4. **Logic Analyzer**: Verify UART timing when in doubt
5. **defmt Logging**: Use appropriate log levels (info, warn, error)

### Binary Protocol Specific

- **Serialize on Desktop First**: Test postcard with std before no_std
- **Print Raw Bytes**: Use defmt::info!("{:02x}", byte) for binary debugging
- **CRC Debugging**: Log both calculated and received CRC values
- **State Machine Visibility**: Log every state transition

### RTIC Considerations

- **Interrupt Priority**: UART should have higher priority than display updates
- **Shared Resource Locks**: Keep lock duration minimal
- **Timer Resolution**: Ensure timeout precision matches requirements

---

## Known Limitations

### RYLR998 LoRa Module

- Maximum payload: 240 bytes (hardware limit)
- UART baud: 115200 (fixed in our setup)
- AT command overhead: ~15 bytes
- Response time: Variable, plan for 50-100ms

### STM32F446RE

- RAM: 128KB (buffer size constraints)
- Flash: 512KB (code size limits)
- Timer resolution: Dependent on prescaler settings

---

## Performance Optimization

### If Retries Are Excessive

**Potential Causes**:
- Timeout too short (ACK hasn't arrived yet)
- Interference on LoRa channel
- Distance too great for current SF/BW settings
- ACK getting lost (check Node 2 transmission)

**Solutions**:
- Increase timeout value
- Test at closer range first
- Verify ACK transmission logic
- Check signal quality (RSSI/SNR)

### If CRC Fails Frequently

**Potential Causes**:
- Corruption during UART transmission
- LoRa packet corruption (poor signal)
- Endianness mismatch
- CRC calculated on wrong data

**Solutions**:
- Verify UART interrupt handler is simple (Week 2 lesson)
- Test at closer range
- Confirm CRC algorithm matches on both nodes
- Log raw bytes before/after serialization

---

## References

### Week 2 Learnings

- [Week 2 MILESTONE](../wk2-lora-sensor-fusion/MILESTONE_LORA_COMMUNICATION.md)
- [Week 2 TROUBLESHOOTING](../wk2-lora-sensor-fusion/TROUBLESHOOTING.md)

### External Resources

- RYLR998 Datasheet
- STM32F446 Reference Manual
- RTIC Book - Troubleshooting
- Postcard GitHub Issues

---

_Last Updated_: [Current Date]
_Status_: Week 3 in progress
