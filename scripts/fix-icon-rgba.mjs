#!/usr/bin/env bun
// Ensure a PNG is encoded as 8-bit RGBA (colorType 6)
// Usage: bun scripts/fix-icon-rgba.mjs <input.png>

import fs from 'node:fs';
import path from 'node:path';
import sharp from 'sharp';

async function main() {
  const input = process.argv[2];
  if (!input) {
    console.error('Usage: bun scripts/fix-icon-rgba.mjs <input.png>');
    process.exit(1);
  }
  const abs = path.resolve(input);
  if (!fs.existsSync(abs)) {
    console.error('File not found:', abs);
    process.exit(1);
  }
  const tmp = abs + '.tmp';
  try {
    // Convert to 8-bit RGBA, keep transparency
    await sharp(abs)
      .ensureAlpha()
      .png({ force: true })
      .toFile(tmp);

    fs.renameSync(tmp, abs);
    console.log('Re-encoded as RGBA:', abs);
  } catch (err) {
    console.error('Failed to convert PNG:', err);
    process.exit(1);
  }
}

main();

