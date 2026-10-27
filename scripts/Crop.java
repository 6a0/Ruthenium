import javax.imageio.ImageIO;
import java.awt.Graphics2D;
import java.awt.RenderingHints;
import java.awt.image.BufferedImage;
import java.io.File;

/** Crop.java out.png x y w h scale in1.png in2.png ... : stacks the zoomed crops vertically */
public class Crop {
    public static void main(String[] args) throws Exception {
        int x = Integer.parseInt(args[1]), y = Integer.parseInt(args[2]);
        int w = Integer.parseInt(args[3]), h = Integer.parseInt(args[4]), s = Integer.parseInt(args[5]);
        int n = args.length - 6;
        BufferedImage out = new BufferedImage(w * s, (h * s + 8) * n, BufferedImage.TYPE_INT_RGB);
        Graphics2D g = out.createGraphics();
        g.setRenderingHint(RenderingHints.KEY_INTERPOLATION, RenderingHints.VALUE_INTERPOLATION_NEAREST_NEIGHBOR);
        for (int i = 0; i < n; i++) {
            BufferedImage in = ImageIO.read(new File(args[6 + i])).getSubimage(x, y, w, h);
            g.drawImage(in, 0, i * (h * s + 8), w * s, h * s, null);
        }
        ImageIO.write(out, "png", new File(args[0]));
    }
}
