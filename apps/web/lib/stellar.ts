import { StellarWalletsKit, Networks } from "@creit.tech/stellar-wallets-kit";
import { StrKey, Transaction } from "@stellar/stellar-sdk";

let kit: StellarWalletsKit | null = null;

export type StellarNetwork = Networks.TESTNET | Networks.PUBLIC;

export const APP_STELLAR_NETWORK: StellarNetwork =
  (process.env.NEXT_PUBLIC_STELLAR_NETWORK as StellarNetwork) ?? Networks.TESTNET;

export function isValidStellarAddress(address: string): boolean {
  return StrKey.isValidEd25519PublicKey(address);
}

export function assertValidStellarAddress(address: string): string {
  if (!isValidStellarAddress(address)) {
    throw new Error("Invalid Stellar account address returned by wallet.");
  }
  return address;
}

export function assertValidTransactionXdr(xdr: string): string {
  try {
    // Parse to ensure shape and network passphrase are valid for this app config.
    new Transaction(xdr, APP_STELLAR_NETWORK);
    return xdr;
  } catch {
    throw new Error("Invalid Stellar transaction XDR.");
  }
}

export function getWalletsKit(): StellarWalletsKit {
  if (!kit) {
    kit = new StellarWalletsKit({
      network: APP_STELLAR_NETWORK,
      selectedWalletId: "freighter",
      modules: ["freighter", "albedo", "xbull"],
    });
  }
  return kit;
}

export async function connectWallet(): Promise<string> {
  if (process.env.NEXT_PUBLIC_E2E === "true") return "GD...CLIENT";
  const walletsKit = getWalletsKit();
  return new Promise<string>((resolve, reject) => {
    walletsKit.openModal({
      onWalletSelected: async () => {
        try {
          walletsKit.closeModal();
          const { address } = await walletsKit.getAddress();
          resolve(assertValidStellarAddress(address));
        } catch (err) {
          reject(err);
        }
      },
      onClosed: () => reject(new Error("Wallet connection cancelled by user.")),
    });
  });
}

export async function disconnectWallet(): Promise<void> {
  if (process.env.NEXT_PUBLIC_E2E === "true") return;
  await getWalletsKit().disconnect();
}

export async function getConnectedWalletAddress(): Promise<string | null> {
  if (process.env.NEXT_PUBLIC_E2E === "true") return "GD...CLIENT";
  try {
    const { address } = await getWalletsKit().getAddress();
    return assertValidStellarAddress(address);
  } catch {
    return null;
  }
}

export async function getWalletNetwork(): Promise<StellarNetwork | null> {
  const walletKit = getWalletsKit() as StellarWalletsKit & {
    getNetwork?: () => Promise<{ network: string }>;
  };

  if (!walletKit.getNetwork) {
    return null;
  }

  try {
    const result = await walletKit.getNetwork();
    const network = result.network;
    if (network === Networks.TESTNET || network === Networks.PUBLIC) {
      return network;
    }
    return null;
  } catch {
    return null;
  }
}

export async function signTransaction(xdr: string): Promise<string> {
  if (process.env.NEXT_PUBLIC_E2E === "true") return xdr;

  const walletsKit = getWalletsKit();
  const validatedXdr = assertValidTransactionXdr(xdr);

  const { signedTxXdr } = await walletsKit.signTransaction(validatedXdr, {
    networkPassphrase: APP_STELLAR_NETWORK,
  });

  return assertValidTransactionXdr(signedTxXdr);
}
