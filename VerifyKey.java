import java.io.*;
import java.nio.file.*;

public class VerifyKey {
    public static void main(String[] args) throws Exception {
        byte[] key_msg = Files.readAllBytes(Path.of(args[0]));
        byte[] ekey = Files.readAllBytes(Path.of(args[1]));
        
        System.out.println("key_msg: " + key_msg.length + " bytes");
        System.out.println("ekey:    " + ekey.length + " bytes");
        
        // Call OmgHax directly
        var omgHax = new com.github.serezhka.jap2lib.OmgHax();
        byte[] aesKey = new byte[16];
        omgHax.decryptAesKey(key_msg, ekey, aesKey);
        
        System.out.print("AES key (Java): ");
        for (byte b : aesKey) System.out.printf("%02x", b & 0xff);
        System.out.println();
    }
}
