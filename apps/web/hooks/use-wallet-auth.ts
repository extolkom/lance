import { useState } from "react";
import { buildSiwsMessage, generateNonce } from "@/lib/siws";
import { signMessage, getConnectedWalletAddress } from "@/lib/stellar";

export const useWalletAuth = () => {
  const [loading, setLoading] = useState(false);

  const login = async () => {
    setLoading(true);
    try {
      const address = await getConnectedWalletAddress();
      if (!address) throw new Error("No wallet connected");

      const domain = typeof window !== "undefined" ? window.location.host : "localhost";

      // FIXED: 'buildSiwsMessage' returns a string, so we don't destructure { message }
      const message = buildSiwsMessage({
        address,
        domain,
        nonce: generateNonce(),
        issuedAt: new Date().toISOString(),
      });

      const signature = await signMessage(message);
      
      // Proceed with your backend verification call here...
      console.log("Message signed:", message, "Signature:", signature);

    } catch (error) {
      console.error("Login failed", error);
    } finally {
      setLoading(false);
    }
  };

  return { login, loading };
};