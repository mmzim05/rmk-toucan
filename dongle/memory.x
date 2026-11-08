MEMORY
{
  /* XIAO nRF52840 with the stock Adafruit nRF52 bootloader. The MBR owns the
   * first 4K and the bootloader starts at 0xF4000. RMK uses its own BLE stack,
   * so it reclaims the SoftDevice region (0x1000-0x27000). */
  FLASH : ORIGIN = 0x00001000, LENGTH = 0xF3000
  /* The bootloader keeps the first 8 bytes of RAM. */
  RAM : ORIGIN = 0x20000008, LENGTH = 255K
}
