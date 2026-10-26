import javax.imageio.ImageIO;
import java.awt.image.BufferedImage;
import java.io.File;

/**
 * Compares two benchmark screenshots.
 * <p>
 * Usage: java scripts/ImageDiff.java a.png b.png [minY=400] [threshold=12] [diffMap.png]
 * <p>
 * Counts pixels (from row minY down, to skip the sky/clouds) whose largest per-channel difference exceeds the
 * threshold, and optionally writes a map with differing pixels in red over a darkened copy of {@code a}.
 * Screenshots from different runs differ slightly in lighting/fog (time of day) and cloud position, so compare
 * against the vanilla-vs-vanilla noise level rather than expecting zero.
 */
public class ImageDiff {
    public static void main(String[] args) throws Exception {
        BufferedImage a = ImageIO.read(new File(args[0]));
        BufferedImage b = ImageIO.read(new File(args[1]));
        int minY = args.length > 2 ? Integer.parseInt(args[2]) : 400;
        int threshold = args.length > 3 ? Integer.parseInt(args[3]) : 12;
        File mapFile = args.length > 4 ? new File(args[4]) : null;

        BufferedImage map = mapFile != null ? new BufferedImage(a.getWidth(), a.getHeight(), BufferedImage.TYPE_INT_RGB) : null;
        int diff = 0, total = 0;

        for (int y = 0; y < a.getHeight(); y++) {
            for (int x = 0; x < a.getWidth(); x++) {
                int p = a.getRGB(x, y), q = b.getRGB(x, y);
                int d = 0;
                for (int s = 0; s < 24; s += 8) {
                    d = Math.max(d, Math.abs(((p >> s) & 0xFF) - ((q >> s) & 0xFF)));
                }
                boolean differs = d > threshold;

                if (y >= minY) {
                    total++;
                    if (differs) diff++;
                }
                if (map != null) {
                    map.setRGB(x, y, differs ? 0xFF0000 : (p >> 2) & 0x3F3F3F);
                }
            }
        }

        System.out.printf("differing pixels (rows >= %d, threshold %d): %d of %d (%.4f%%)%n",
                minY, threshold, diff, total, 100.0 * diff / total);
        if (map != null) {
            ImageIO.write(map, "png", mapFile);
        }
    }
}
