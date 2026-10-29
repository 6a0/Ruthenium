import javax.imageio.ImageIO;
import java.awt.image.BufferedImage;
import java.io.File;

/**
 * What a switch changes in frozen close-ups whose shots of one setting differ anyway (glint, the first-person arm):
 * {@code java scripts/StableDiff.java <threshold> <map.png> shot0 shot1 ...}, even shots one setting, odd shots the other (as
 * {@code bench.closeToggle} takes them). Counts pixels that are equal (difference below the threshold, max over channels) in all
 * shots of each setting but differ across the settings, with a histogram of their differences, and writes a map: red such pixels,
 * blue pixels that differ across the settings but also within one. ImageDiff's "threshold 1" is a threshold of 2 here.
 */
public class StableDiff {
    public static void main(String[] args) throws Exception {
        int t = Integer.parseInt(args[0]);
        int n = args.length - 2;
        BufferedImage[] shots = new BufferedImage[n];
        for (int i = 0; i < n; i++) shots[i] = ImageIO.read(new File(args[i + 2]));
        int w = shots[0].getWidth(), h = shots[0].getHeight();
        BufferedImage map = new BufferedImage(w, h, BufferedImage.TYPE_INT_RGB);
        int stable = 0, across = 0;
        int[] histogram = new int[256];
        for (int y = 0; y < h; y++) for (int x = 0; x < w; x++) {
            int a = shots[0].getRGB(x, y), b = shots[1].getRGB(x, y);
            boolean same = true;
            for (int i = 2; i < n && same; i++) same = diff(shots[i].getRGB(x, y), i % 2 == 0 ? a : b) < t;
            int g = ((a >> 16 & 255) + (a >> 8 & 255) + (a & 255)) / 12;
            int rgb = g << 16 | g << 8 | g;
            if (diff(a, b) >= t) {
                across++;
                if (same) { stable++; histogram[diff(a, b)]++; rgb = 0xFF0000; } else rgb = 0x0000FF;
            }
            map.setRGB(x, y, rgb);
        }
        StringBuilder sb = new StringBuilder();
        for (int i = 0; i < 256; i++) if (histogram[i] > 0) sb.append(' ').append(i).append('x').append(histogram[i]);
        System.out.printf("%d shots, difference >= %d: %d px differ across the settings, %d stable within each setting;%s%n", n, t, across, stable, sb);
        ImageIO.write(map, "png", new File(args[1]));
    }

    static int diff(int p, int q) {
        return Math.max(Math.abs((p >> 16 & 255) - (q >> 16 & 255)), Math.max(Math.abs((p >> 8 & 255) - (q >> 8 & 255)), Math.abs((p & 255) - (q & 255))));
    }
}
