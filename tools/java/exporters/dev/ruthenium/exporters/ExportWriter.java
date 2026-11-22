/*
 * Original Ruthenium code, GPL-3.0-or-later.
 */
package dev.ruthenium.exporters;

import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;

/**
 * Little-endian writer for Ruthenium export files.
 *
 * <p>Every file starts with the magic {@code RTHX}, a u32 format version, and the
 * export's name as a string. Strings are a u16 byte length followed by UTF-8.
 */
final class ExportWriter {
    private final ByteArrayOutputStream bytes = new ByteArrayOutputStream();

    /** A file with its header; a {@code null} name makes a headerless fragment. */
    ExportWriter(String name, int version) {
        if (name != null) {
            bytes.writeBytes("RTHX".getBytes(StandardCharsets.US_ASCII));
            u32(version);
            string(name);
        }
    }

    void bytes(byte[] b) {
        bytes.writeBytes(b);
    }

    byte[] payload() {
        return bytes.toByteArray();
    }

    void u8(int v) {
        bytes.write(v);
    }

    void u32(int v) {
        bytes.write(v);
        bytes.write(v >>> 8);
        bytes.write(v >>> 16);
        bytes.write(v >>> 24);
    }

    void f32(float v) {
        u32(Float.floatToRawIntBits(v));
    }

    void f64(double v) {
        long bits = Double.doubleToRawLongBits(v);
        u32((int) bits);
        u32((int) (bits >>> 32));
    }

    void string(String s) {
        byte[] utf8 = s.getBytes(StandardCharsets.UTF_8);
        if (utf8.length > 0xFFFF) {
            throw new IllegalArgumentException("string too long: " + s);
        }
        bytes.write(utf8.length);
        bytes.write(utf8.length >>> 8);
        bytes.writeBytes(utf8);
    }

    void writeTo(Path path) throws IOException {
        Files.write(path, bytes.toByteArray());
    }
}
