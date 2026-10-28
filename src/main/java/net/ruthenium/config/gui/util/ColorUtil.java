package net.ruthenium.config.gui.util;

public final class ColorUtil {

    private ColorUtil() {
    }

    public static int floatToInt(float f) {
        return (int) (f * 255.0f) & 0xFF;
    }

    public static class ARGB {
        public static int pack(float r, float g, float b, float a) {
            return floatToInt(a) << 24 | floatToInt(r) << 16 | floatToInt(g) << 8 | floatToInt(b);
        }

        public static int multiplyAlpha(int color, float m) {
            int newA = floatToInt(((color >>> 24) & 0xFF) / 255.0f * m);
            return (color & 0x00FFFFFF) | newA << 24;
        }
    }
}
