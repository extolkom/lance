import { Horizon, Networks } from "@stellar/stellar-sdk";

export type StellarNetwork = Networks.PUBLIC | Networks.TESTNET | string;

export const APP_STELLAR_NETWORK = (process.env.NEXT_PUBLIC_STELLAR_NETWORK || "testnet").toUpperCase() === "PUBLIC" 
  ? Networks.PUBLIC 
  : Networks.TESTNET;

const HORIZON_URL = process.env.NEXT_PUBLIC_HORIZON_URL || "https://horizon-testnet.stellar.org";
export const horizonServer = new Horizon.Server(HORIZON_URL);

// --- Validation Utils ---
export function isValidStellarAddress(address: string): boolean {
  try {
    return /^[G][A-Z2-7]{55}$/.test(address);
  } catch {
    return false;
  }
}

export function assertValidStellarAddress(address: string): string {
  if (!isValidStellarAddress(address)) {
    throw new Error("Invalid Stellar address");
  }
  return address;
}

// --- Network & Balance Utils (From main) ---
export function getHorizonUrl(network: StellarNetwork): string {
  return network === Networks.PUBLIC
    ? "https://horizon.stellar.org"
    : "https://horizon-testnet.stellar.org";
}

export async function getXlmBalance(address: string): Promise<string | null> {
  if (process.env.NEXT_PUBLIC_E2E === "true") return "1000.0000000";

  const validatedAddress = assertValidStellarAddress(address);
  const server = new Horizon.Server(getHorizonUrl(APP_STELLAR_NETWORK));

  try {
    const account = await server.loadAccount(validatedAddress);
    const nativeBalance = account.balances.find(
      (balance): balance is Horizon.HorizonApi.BalanceLineNative =>
        balance.asset_type === "native",
    );
    return nativeBalance?.balance ?? null;
  } catch {
    return null;
  }
}

// --- Wallet Auth & Connection Utils (From your branch) ---
export function getWalletNetwork(): string {
  return APP_STELLAR_NETWORK === Networks.PUBLIC ? "public" : "testnet";
}

export function disconnectWallet(): void {
  if (typeof window !== "undefined") {
    localStorage.removeItem("wallet_address");
    localStorage.removeItem("wallet_type");
    window.dispatchEvent(new Event("storage"));
  }
}

export function getWalletsKit() {
  return {}; 
}

export async function getConnectedWalletAddress(): Promise<string | null> {
  if (typeof window !== "undefined") {
    return localStorage.getItem("wallet_address") || null;
  }
  return null;
}

export async function connectWallet(): Promise<string> {
  return ""; 
}

export async function signTransaction(xdr: string): Promise<string> {
  return xdr; 
}

export async function signMessage(message: string): Promise<string> {
  return "";
}