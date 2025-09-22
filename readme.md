Here’s a clean **`README.md`** for your project. It explains setup, configuration, how to run, and what the bot does. You can drop this into the root of your repo.

---

````markdown
# Rust DEX Arbitrage Bot

A simple arbitrage opportunity detector written in **Rust** using [`ethers-rs`](https://github.com/gakonst/ethers-rs), [`warp`](https://github.com/seanmonstar/warp) for WebSockets, and [`tokio`](https://tokio.rs) for async tasks.  
It scans multiple DEXes on Polygon (QuickSwap, SushiSwap, DFYN, QuickSwapV2, etc.), compares token prices, simulates round-trip swaps, subtracts dynamic gas costs (estimated in USDC), and broadcasts profitable opportunities.

---

## ✨ Features
- Queries prices from multiple **Uniswap V2-style DEX routers**.
- Calculates arbitrage opportunities between buy/sell DEX pairs.
- Estimates **gas costs in USDC** dynamically using chain gas price + router pricing.
- Runs a **WebSocket server** to broadcast new opportunities to connected clients.
- Easily extendable with new tokens and DEXes via `token.json` and config.

---

## 📦 Requirements

- [Rust](https://www.rust-lang.org/tools/install) (latest stable recommended).
- Polygon RPC URL (free from [Alchemy](https://www.alchemy.com/), [Infura](https://infura.io/), or public node).
- Token list file (`token.json`) containing ERC-20 metadata.
- On Windows: OpenSSL dev libraries may be needed (or build with `rustls-tls` features to avoid OpenSSL).

---

## ⚙️ Setup

1. **Clone the repo**
   ```bash
   git clone https://github.com/yourname/rust-bot.git
   cd rust-bot
````

2. **Install dependencies**

   ```bash
   cargo build
   ```

3. **Configure environment variables**
   Create a `.env` file or export manually:

   ```bash
   # Polygon RPC endpoint
   RPC_URL=https://polygon-rpc.com

   # Native token address (Polygon WMATIC)
   NATIVE_ADDRESS=0x0d500B1d8E8eF31E21C99d1Db9A6444d3ADf1270

   # Router used to price native→USDC (QuickSwap by default)
   PRICE_ROUTER=0xa5E0829CaCED8fFDD4De3c43696c57F7D7A678ff

   # Override if native decimals are not 18
   NATIVE_DECIMALS=18
   ```

4. **Add tokens**
   Edit `token.json` to include tokens you want to scan (apart from USDC):

   ```json
   [
     { "symbol": "WETH", "address": "0x7ceB23fD6bC0adD59E62ac25578270cFf1b9f619", "decimals": 18 },
     { "symbol": "USDC", "address": "0x2791Bca1f2de4661ED88A30C99A7a9449Aa84174", "decimals": 6 },
     { "symbol": "WBTC", "address": "0x1BFD67037B42Cf73acF2047067bd4F2C47D9BfD6", "decimals": 8 },
     { "symbol": "WMATIC", "address": "0x0d500B1d8E8eF31E21C99d1Db9A6444d3ADf1270", "decimals": 18 },
     { "symbol": "DAI", "address": "0x8f3Cf7ad23Cd3CaDbD9735AFf958023239c6A063", "decimals": 18 },
     { "symbol": "USDT", "address": "0xc2132D05D31c914a87C6611C10748AEb04B58e8F", "decimals": 6 }
   ]
   ```

---

## 🚀 Run

Start the bot:

```bash
cargo run
```

By default it runs a WebSocket server on:

```
ws://127.0.0.1:3030/ws
```

Any client can connect and will receive JSON messages with opportunities:

```json
[
  {
    "token": "WETH",
    "buy_dex": "SushiSwap",
    "sell_dex": "QuickSwap",
    "buy_price_usdc": "1299.42",
    "sell_price_usdc": "1310.11",
    "estimated_profit_usdc": "8.95"
  }
]
```

---

## 🛠 Extend

* **Add new DEX routers** → update `dexes` list in `main.rs`.
* **Change trade size / min profit filter** → edit constants in `main.rs`.
* **Integrate execution logic** → currently only simulates opportunities. You can extend it to send signed transactions if you want to execute arbitrage.

---
