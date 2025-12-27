#!/usr/bin/env python3
"""
Range Test Visualization Script
Plots RSSI, SNR, and Packet Loss vs Distance for LoRa testing

Usage: python3 plot_range_test.py
Output: range_test_results.png
"""

import matplotlib.pyplot as plt
import numpy as np

# Test data from 2025-12-27
distances = [15, 30, 60, 100, 150, 400, 600]  # meters
rssi = [-45, -62, -72, -82, -91, -100, -107]  # dBm
snr = [13, 13, 12, 11, 4, -2, -6]  # dB
packet_loss = [0, 1, 1, 1, 2, 2, 5]  # percentage

# Create figure with 3 subplots
fig, (ax1, ax2, ax3) = plt.subplots(3, 1, figsize=(10, 10))
fig.suptitle('LoRa Range Test Results - Suburban Environment (No Line of Sight)',
             fontsize=14, fontweight='bold')

# Plot 1: RSSI vs Distance
ax1.plot(distances, rssi, 'b-o', linewidth=2, markersize=8, label='Measured RSSI')
ax1.axhline(y=-120, color='r', linestyle='--', linewidth=1, alpha=0.5, label='Sensitivity Limit (~-120 dBm)')
ax1.grid(True, alpha=0.3)
ax1.set_xlabel('Distance (m)', fontsize=11)
ax1.set_ylabel('RSSI (dBm)', fontsize=11)
ax1.set_title('Signal Strength vs Distance', fontsize=12, fontweight='bold')
ax1.legend(loc='lower left')
ax1.set_xlim(0, 650)
ax1.set_ylim(-125, -40)

# Add annotations for key points
ax1.annotate('Strong signal\nthrough walls', xy=(15, -45), xytext=(50, -50),
            arrowprops=dict(arrowstyle='->', color='green', lw=1.5),
            fontsize=9, color='green')
ax1.annotate('Approaching\nsensitivity limit', xy=(600, -107), xytext=(450, -115),
            arrowprops=dict(arrowstyle='->', color='red', lw=1.5),
            fontsize=9, color='red')

# Plot 2: SNR vs Distance
ax2.plot(distances, snr, 'g-s', linewidth=2, markersize=8, label='Measured SNR')
ax2.axhline(y=0, color='k', linestyle='-', linewidth=1, alpha=0.3, label='0 dB (noise floor)')
ax2.axhspan(-10, 0, alpha=0.2, color='yellow', label='Negative SNR zone')
ax2.grid(True, alpha=0.3)
ax2.set_xlabel('Distance (m)', fontsize=11)
ax2.set_ylabel('SNR (dB)', fontsize=11)
ax2.set_title('Signal-to-Noise Ratio vs Distance', fontsize=12, fontweight='bold')
ax2.legend(loc='upper right')
ax2.set_xlim(0, 650)
ax2.set_ylim(-10, 15)

# Add annotation about negative SNR
ax2.annotate('LoRa still works\nwith negative SNR!', xy=(400, -2), xytext=(200, -7),
            arrowprops=dict(arrowstyle='->', color='orange', lw=1.5),
            fontsize=9, color='orange', fontweight='bold')

# Plot 3: Packet Loss vs Distance
ax3.plot(distances, packet_loss, 'r-^', linewidth=2, markersize=8, label='Packet Loss')
ax3.fill_between(distances, 0, packet_loss, alpha=0.3, color='red')
ax3.grid(True, alpha=0.3)
ax3.set_xlabel('Distance (m)', fontsize=11)
ax3.set_ylabel('Packet Loss (%)', fontsize=11)
ax3.set_title('Packet Loss vs Distance', fontsize=12, fontweight='bold')
ax3.legend(loc='upper left')
ax3.set_xlim(0, 650)
ax3.set_ylim(0, 10)

# Add success rate annotations
success_rate_600m = 100 - packet_loss[-1]
ax3.text(600, 6, f'{success_rate_600m}% success\nat 600m',
         fontsize=10, color='green', fontweight='bold',
         bbox=dict(boxstyle='round', facecolor='lightgreen', alpha=0.5))

# Add test conditions text box
conditions_text = """Test Conditions:
• Location: Suburban residential
• Environment: No line of sight
• Obstacles: Buildings, trees, walls
• Weather: 15°C, light clouds
• Module: RYLR998 (915 MHz)
• Date: 2025-12-27"""

fig.text(0.02, 0.02, conditions_text, fontsize=9,
         bbox=dict(boxstyle='round', facecolor='wheat', alpha=0.5),
         verticalalignment='bottom', family='monospace')

# Add statistics text box
stats_text = f"""Key Results:
• Maximum range: 600m
• Success rate at 600m: {100-packet_loss[-1]}%
• RSSI range: {rssi[0]} to {rssi[-1]} dBm
• SNR range: {snr[-1]} to {snr[0]} dB
• Works with negative SNR!"""

fig.text(0.98, 0.02, stats_text, fontsize=9,
         bbox=dict(boxstyle='round', facecolor='lightblue', alpha=0.5),
         verticalalignment='bottom', horizontalalignment='right',
         family='monospace')

plt.tight_layout(rect=[0, 0.08, 1, 0.96])
plt.savefig('range_test_results.png', dpi=300, bbox_inches='tight')
print("✅ Graph saved as: range_test_results.png")
print(f"\nSummary:")
print(f"  Maximum distance tested: {distances[-1]}m")
print(f"  RSSI at max distance: {rssi[-1]} dBm")
print(f"  SNR at max distance: {snr[-1]} dB")
print(f"  Success rate at max distance: {100-packet_loss[-1]}%")
print(f"  Overall: Excellent LoRa performance through obstacles!")

# Optional: Show the plot
# plt.show()
