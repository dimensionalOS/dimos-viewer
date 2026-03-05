#!/usr/bin/env python3
"""
Simple LCM subscriber test script to verify TwistStamped messages from dimos-viewer keyboard controls.
Run this alongside the dimos-viewer to see keyboard input being published over LCM.
"""

import lcm
import sys
import time
from struct import unpack

def decode_twist_stamped(data):
    """Decode TwistStamped LCM message manually"""
    offset = 0
    
    # Skip fingerprint hash
    offset += 8
    
    # Header
    seq = unpack('>I', data[offset:offset+4])[0]
    offset += 4
    sec = unpack('>i', data[offset:offset+4])[0]
    offset += 4
    nsec = unpack('>i', data[offset:offset+4])[0]
    offset += 4
    
    # Frame ID string
    str_len = unpack('>I', data[offset:offset+4])[0]
    offset += 4
    frame_id = data[offset:offset+str_len-1].decode('utf-8')  # -1 for null terminator
    offset += str_len
    
    # Twist: 6 doubles (linear x,y,z + angular x,y,z)
    linear_x = unpack('>d', data[offset:offset+8])[0]
    offset += 8
    linear_y = unpack('>d', data[offset:offset+8])[0]
    offset += 8
    linear_z = unpack('>d', data[offset:offset+8])[0]
    offset += 8
    angular_x = unpack('>d', data[offset:offset+8])[0]
    offset += 8
    angular_y = unpack('>d', data[offset:offset+8])[0]
    offset += 8
    angular_z = unpack('>d', data[offset:offset+8])[0]
    
    return {
        'seq': seq,
        'timestamp': f"{sec}.{nsec:09d}",
        'frame_id': frame_id,
        'linear': [linear_x, linear_y, linear_z],
        'angular': [angular_x, angular_y, angular_z]
    }

def on_twist_message(channel, data):
    """Handle TwistStamped LCM message"""
    try:
        twist = decode_twist_stamped(data)
        print(f"[{twist['timestamp']}] Twist from {twist['frame_id']}:")
        print(f"  Linear:  [{twist['linear'][0]:6.2f}, {twist['linear'][1]:6.2f}, {twist['linear'][2]:6.2f}]")
        print(f"  Angular: [{twist['angular'][0]:6.2f}, {twist['angular'][1]:6.2f}, {twist['angular'][2]:6.2f}]")
        print()
    except Exception as e:
        print(f"Error decoding twist message: {e}")
        print(f"Raw data length: {len(data)} bytes")

def main():
    print("🎮 DimOS Keyboard Control Test - LCM Subscriber")
    print("Listening for TwistStamped messages on /cmd_vel...")
    print("Start dimos-viewer and press WASD keys to see output here.")
    print("Press Ctrl+C to exit.\n")
    
    lc = lcm.LCM()
    subscription = lc.subscribe("/cmd_vel#geometry_msgs.TwistStamped", on_twist_message)
    
    try:
        while True:
            lc.handle()
    except KeyboardInterrupt:
        print("\nExiting...")
        lc.unsubscribe(subscription)

if __name__ == "__main__":
    main()
