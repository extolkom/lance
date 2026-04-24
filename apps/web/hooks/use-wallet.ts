import { useState, useCallback, useEffect } from "react";
import { 
  StellarWalletsKit, 
  WalletNetwork, 
  allowAll, 
  FreighterModule, 
  AlbedoModule, 
  XBullModule 
} from "@creit.tech/stellar-wallets-kit";
import { useAuthStore } from "@/lib/store/use-auth-store";
import { toast } from "sonner";

// Initialize the kit
const kit = new StellarWalletsKit({
  network: WalletNetwork.TESTNET,
  modules: [
    new FreighterModule(),
    new AlbedoModule(),
    new XBullModule(),
  ],
});

export function useWallet() {
  const { login, logout, user, isLoggedIn } = useAuthStore();
  const [isConnecting, setIsConnecting] = useState(false);

  const connect = useCallback(async () => {
    setIsConnecting(true);
    try {
      // 1. Get address and network from wallet
      const { address } = await kit.getAddress();
      const walletNetwork = await kit.getNetwork();
      
      const expectedNetwork = process.env.NEXT_PUBLIC_STELLAR_NETWORK?.toUpperCase() || "TESTNET";
      
      if (walletNetwork !== walletNetwork) { // This is a placeholder for actual comparison logic if needed
          // Actually kit.getNetwork() returns the network name
      }

      if (walletNetwork.toUpperCase() !== expectedNetwork) {
        toast.warning(`Network Mismatch: App is on ${expectedNetwork} but wallet is on ${walletNetwork}`, {
          duration: 10000,
        });
      }

      // 2. Fetch challenge from backend
      const challengeResp = await fetch(`${process.env.NEXT_PUBLIC_API_URL}/v1/auth/challenge`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(address),
      });
      
      if (!challengeResp.ok) throw new Error("Failed to fetch auth challenge");
      const { challenge } = await challengeResp.json();

      // 3. Sign challenge
      const { result: signature } = await kit.signAuthEntry({
        entry: challenge,
        address,
      });

      // 4. Verify signature on backend
      const verifyResp = await fetch(`${process.env.NEXT_PUBLIC_API_URL}/v1/auth/verify`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ address, signature }),
      });

      if (!verifyResp.ok) throw new Error("Signature verification failed");
      const { token } = await verifyResp.json();

      // 5. Update store
      login(
        {
          address,
          token,
          name: address.slice(0, 4) + "..." + address.slice(-4),
          email: "",
        },
        "client" // Default to client for now, or fetch from profile
      );

      toast.success("Wallet connected successfully");
    } catch (error: any) {
      console.error("Wallet connection error:", error);
      toast.error(error.message || "Failed to connect wallet");
    } finally {
      setIsConnecting(false);
    }
  }, [login]);

  // Poll for account switches (for wallets that don't emit events)
  useEffect(() => {
    if (!isLoggedIn || !address) return;

    const interval = setInterval(async () => {
      try {
        const { address: currentAddress } = await kit.getAddress();
        if (currentAddress !== address) {
          logout();
          toast.info("Account switched in wallet. Please reconnect.");
        }
      } catch (e) {
        // Wallet might be locked or disconnected
      }
    }, 3000);

    return () => clearInterval(interval);
  }, [isLoggedIn, address, logout]);

  const disconnect = useCallback(() => {
    logout();
    toast.info("Wallet disconnected");
  }, [logout]);

  return {
    connect,
    disconnect,
    isConnecting,
    isLoggedIn,
    address: user?.address,
  };
}
