import type { NextConfig } from "next";
import path from "path";

const nextConfig: NextConfig = {
  turbopack: {
    root: path.resolve(process.cwd(), "../../"),
  },

  webpack(config, { isServer, webpack }) {
    if (!isServer) {
      // ── 1. Ignore sodium-native entirely on the client ──────────────────────
      // stellar-base pulls in sodium-native as an *optional* Node.js C++ addon.
      // It is never needed in the browser — the SDK falls back to tweetnacl
      // automatically. Ignoring it here prevents Webpack from emitting
      // "Critical dependency" warnings and avoids bundling a native module.
      config.plugins.push(
        new webpack.IgnorePlugin({
          resourceRegExp: /^sodium-native$/,
        }),
        // Also ignore the generic require-addon helper used by some native modules
        new webpack.IgnorePlugin({
          resourceRegExp: /^require-addon$/,
        }),
      );

      // ── 2. Node.js built-in fallbacks ────────────────────────────────────────
      // stellar-sdk references several Node built-ins. In the browser these
      // should either be polyfilled or silently stubbed out.
      config.resolve.fallback = {
        ...config.resolve.fallback,
        fs: false,
        path: false,
        crypto: false,
        net: false,
        tls: false,
        dns: false,
      };
    }

    return config;
  },
};

export default nextConfig;
