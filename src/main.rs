use ethers::prelude::*;
use std::sync::Arc;
use std::time::Duration;
use serde::{Deserialize, Serialize};
use anyhow::Result;
use rust_decimal::Decimal;
use std::str::FromStr;
use futures::future::join_all;
use futures::{SinkExt, StreamExt};
use tokio::sync::broadcast;
use warp::Filter;

abigen!(
    UniswapV2Router,
    r#"[ function getAmountsOut(uint256 amountIn, address[] path) external view returns (uint256[] amounts) ]"#,
);

#[derive(Debug, Clone, Deserialize)]
struct TokenInfo {
    symbol: String,
    address: String,
    decimals: u8,
}

#[derive(Debug, Serialize, Clone, PartialEq, Eq)]
struct Opportunity {
    token: String,
    buy_dex: String,
    sell_dex: String,
    buy_price_usdc: Decimal,
    sell_price_usdc: Decimal,
    estimated_profit_usdc: Decimal,
}

#[derive(Debug, Clone)]
struct Dex {
    name: String,
    router: Address,
    /// estimated gas units for a swap on this dex (user-tunable)
    estimated_gas_units: u64,
}

/// Convert Decimal -> wei string (no rounding)
fn decimal_to_wei_str(amount: Decimal, decimals: u32) -> String {
    let s = amount.normalize().to_string();
    if s.contains('.') {
        let parts: Vec<&str> = s.split('.').collect();
        let int_part = parts.get(0).copied().unwrap_or("0");
        let frac_part = parts.get(1).copied().unwrap_or("");
        let mut frac = frac_part.to_string();
        if frac.len() > decimals as usize {
            frac = frac[..decimals as usize].to_string();
        } else if frac.len() < decimals as usize {
            frac.push_str(&"0".repeat(decimals as usize - frac.len()));
        }
        let int_clean = if int_part.is_empty() { "0" } else { int_part };
        format!("{}{}", int_clean, frac)
    } else {
        format!("{}{}", s, "0".repeat(decimals as usize))
    }
}

/// Estimate gas cost in USDC for a given gas_units total.
///
/// Steps:
/// 1. Fetch gas_price from provider (wei per gas).
/// 2. gas_cost_native_wei = gas_price * gas_units.
/// 3. Convert native->USDC via Uniswap `getAmountsOut(gas_cost_native_wei, [native, usdc])`.
async fn estimate_gas_usdc<P: JsonRpcClient + 'static>(
    provider: Arc<Provider<P>>, // take Arc so we can pass it into `UniswapV2Router::new`
    gas_units: u64,
    native_address: Address,
    usdc_address: Address,
    _native_decimals: u32,
    usdc_decimals: u32,
    // router to use to price native->usdc (must be a router that supports path [native, usdc])
    price_router: Address,
) -> Option<Decimal> {
    // 1) gas_price
    let gas_price = match provider.get_gas_price().await {
        Ok(gp) => gp, // U256
        Err(_) => return None,
    };

    // gas_cost_native_wei = gas_price * gas_units
    let gas_units_u256 = U256::from(gas_units);
    let gas_cost_native_wei = gas_price.checked_mul(gas_units_u256)?;

    // Use router to get amounts out: native -> USDC
    let router = UniswapV2Router::new(price_router, provider.clone());
    let res = router
        .get_amounts_out(gas_cost_native_wei, vec![native_address, usdc_address])
        .call()
        .await;
    match res {
        Ok(amounts) => {
            if let Some(last) = amounts.last() {
                let s = last.to_string();
                // parse according to usdc_decimals
                let dec = if s.len() > (usdc_decimals as usize) {
                    let pos = s.len() - usdc_decimals as usize;
                    let whole = &s[..pos];
                    let frac = &s[pos..];
                    Decimal::from_str(&format!("{}.{}", whole, frac)).ok()
                } else {
                    let padded = format!("{:0>width$}", s, width = usdc_decimals as usize);
                    Decimal::from_str(&format!("0.{}", padded)).ok()
                };
                dec
            } else {
                None
            }
        }
        Err(_) => None,
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // CONFIG
    let rpc = std::env::var("RPC_URL").unwrap_or_else(|_| "https://polygon-rpc.com".to_string());
    let provider = Provider::<Http>::try_from(rpc)?.interval(Duration::from_millis(1500));
    let provider = Arc::new(provider);

    // NATIVE token on chain (polygon WMATIC) - change if using another chain
    // WMATIC on Polygon: 0x0d500B1d8E8eF31E21C99d1Db9A6444d3ADf1270
    let native_addr_env = std::env::var("NATIVE_ADDRESS")
        .unwrap_or_else(|_| "0x0d500B1d8E8eF31E21C99d1Db9A6444d3ADf1270".to_string());
    let native_address: Address = native_addr_env.parse()?;

    // For converting gas (native -> USDC) we pick a router to price native -> USDC (prefer a large/liquid router)
    // (You can set PRICE_ROUTER env to any router address)
    let price_router_addr = std::env::var("PRICE_ROUTER")
        .unwrap_or_else(|_| "0xa5E0829CaCED8fFDD4De3c43696c57F7D7A678ff".to_string()); // QuickSwap by default
    let price_router: Address = price_router_addr.parse()?;

    // DEX list (you can add more entries). estimated_gas_units is tunable per DEX.
    // Typical swap gas on Polygon might be ~80k-200k depending on router and complexity.
    let dexes: Vec<Dex> = vec![
        Dex { name: "QuickSwap".to_string(), router: "0xa5E0829CaCED8fFDD4De3c43696c57F7D7A678ff".parse()?, estimated_gas_units: 100_000 },
        Dex { name: "SushiSwap".to_string(), router: "0x1b02da8cb0d097eb8d57a175b88c7d8b47997506".parse()?, estimated_gas_units: 120_000 },
        Dex { name: "DFYN".to_string(), router: "0xA102072A4C07F06EC3B4900FDC4C7B80b6f4D1b1".parse()?, estimated_gas_units: 110_000 },
        Dex { name: "QuickSwapV2".to_string(), router: "0x5757371414417b8c6caad45baef941abc7d3ab32".parse()?, estimated_gas_units: 100_000 },
    ];

    // user parameters
    let trade_size_usdc = Decimal::from_str("1000")?;
    let min_profit_usdc = Decimal::from_str("5")?;

    // load tokens
    let token_json = std::fs::read_to_string("token.json")?;
    let tokens: Vec<TokenInfo> = serde_json::from_str(&token_json)?;
    let usdc_tok = tokens.iter().find(|t| t.symbol == "USDC").expect("USDC must be in tokenlist");
    let usdc_addr: Address = usdc_tok.address.parse()?;
    let usdc_dec = usdc_tok.decimals as u32;

    // native decimals - assume 18 (if not, set env or derive)
    let native_decimals: u32 = std::env::var("NATIVE_DECIMALS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(18);

    // broadcast channel for WebSocket
    let (tx, _rx) = broadcast::channel::<String>(16);

    // clone for polling task
    let tx_poll = tx.clone();
    let provider_poll = provider.clone();
    let tokens_poll = tokens.clone();
    let dexes_poll = dexes.clone();
    let price_router_poll = price_router;
    let native_address_poll = native_address;

    // spawn polling loop
    tokio::spawn(async move {
        let poll_interval = Duration::from_secs(10);
        // persist previously seen opportunities across polls (so duplicates across polls are ignored)
        let mut seen_ops: Vec<Opportunity> = Vec::new();

        loop {
            let mut new_ops_this_poll: Vec<Opportunity> = Vec::new();

            for token in tokens_poll.iter().filter(|t| t.symbol != "USDC") {
                let token_addr: Address = match token.address.parse() { Ok(a) => a, Err(_) => continue };
                let token_dec = token.decimals as u32;
                let amount_one = U256::exp10(token_dec as usize);
                let path = vec![token_addr, usdc_addr];

                // query all dex prices
                let mut handles = Vec::new();
                for dex in dexes_poll.iter() {
                    let router_addr = dex.router;
                    let prov = provider_poll.clone();
                    let p = path.clone();
                    let name_cl = dex.name.clone();
                    handles.push(tokio::spawn(async move {
                        let client = UniswapV2Router::new(router_addr, prov);
                        let res = client.get_amounts_out(amount_one, p).call().await;
                        (name_cl, router_addr, res)
                    }));
                }

                let results = join_all(handles).await;
                let mut prices: Vec<(String, Decimal, U256, Address)> = Vec::new();
                for r in results {
                    if let Ok((dex_name, router_addr, Ok(amounts))) = r {
                        if let Some(out) = amounts.last() {
                            let out_str = out.to_string();
                            let out_dec = if out_str.len() > (usdc_dec as usize) {
                                let pos = out_str.len() - usdc_dec as usize;
                                let whole = &out_str[..pos];
                                let frac = &out_str[pos..];
                                Decimal::from_str(&format!("{}.{}", whole, frac)).unwrap_or(Decimal::ZERO)
                            } else {
                                let padded = format!("{:0>width$}", out_str, width = usdc_dec as usize);
                                Decimal::from_str(&format!("0.{}", padded)).unwrap_or(Decimal::ZERO)
                            };
                            prices.push((dex_name, out_dec, *out, router_addr));
                        }
                    }
                }

                if prices.len() < 2 { continue; }
                prices.sort_by(|a, b| a.1.cmp(&b.1));
                let (buy_name, buy_price_usdc, _buy_out_u256, _buy_router) = prices.first().unwrap().clone();
                let (sell_name, sell_price_usdc, _sell_out_u256, sell_router_addr) = prices.last().unwrap().clone();

                if buy_price_usdc.is_zero() { continue; }
                let token_amount = trade_size_usdc / buy_price_usdc;
                let token_amount_wei_str = decimal_to_wei_str(token_amount, token_dec);
                let token_amount_wei = match U256::from_dec_str(&token_amount_wei_str) { Ok(v) => v, Err(_) => continue };

                // simulate sell (try using sell_router first)
                let sell_client = UniswapV2Router::new(sell_router_addr, provider_poll.clone());
                let sell_amounts = sell_client.get_amounts_out(token_amount_wei, vec![token_addr, usdc_addr]).call().await;
                let sell_received_usdc = match sell_amounts { Ok(v) => v.last().cloned().unwrap_or_default(), Err(_) => U256::zero() };

                let sell_str = sell_received_usdc.to_string();
                let sell_received_dec = if sell_str.len() > (usdc_dec as usize) {
                    let pos = sell_str.len() - usdc_dec as usize;
                    Decimal::from_str(&format!("{}.{}", &sell_str[..pos], &sell_str[pos..])).unwrap_or(Decimal::ZERO)
                } else {
                    let padded = format!("{:0>width$}", sell_str, width = usdc_dec as usize);
                    Decimal::from_str(&format!("0.{}", padded)).unwrap_or(Decimal::ZERO)
                };

                let gross = sell_received_dec - trade_size_usdc;

                // --- dynamic gas estimate ---
                // find dex entries for buy and sell to get estimated gas units
                let buy_dex_entry = dexes_poll.iter().find(|d| d.name == *buy_name).cloned();
                let sell_dex_entry = dexes_poll.iter().find(|d| d.name == *sell_name).cloned();

                let mut total_gas_usdc = None;
                if let (Some(bd), Some(sd)) = (buy_dex_entry, sell_dex_entry) {
                    // round-trip gas units: buy + sell (plus small overhead)
                    let gas_units_total = bd.estimated_gas_units.saturating_add(sd.estimated_gas_units).saturating_add(20_000);
                    total_gas_usdc = estimate_gas_usdc(
                        provider_poll.clone(),         // pass Arc<Provider>
                        gas_units_total,
                        native_address_poll,
                        usdc_addr,
                        native_decimals,
                        usdc_dec,
                        price_router_poll,
                    ).await;
                }

                // If dynamic estimation failed, fallback to a small default (e.g., 2 USDC)
                let simulated_gas_usdc = total_gas_usdc.unwrap_or_else(|| Decimal::from_str("2").unwrap());

                let net = if gross > simulated_gas_usdc { gross - simulated_gas_usdc } else { Decimal::ZERO };

                if net >= min_profit_usdc && (sell_price_usdc / buy_price_usdc) < Decimal::from_str("100").unwrap() {
                    let opp = Opportunity {
                        token: token.symbol.clone(),
                        buy_dex: buy_name.clone(),
                        sell_dex: sell_name.clone(),
                        buy_price_usdc,
                        sell_price_usdc,
                        estimated_profit_usdc: net,
                    };
                    // check if we've already seen this exact opportunity before (prevent duplicates)
                    if !seen_ops.contains(&opp) && !new_ops_this_poll.contains(&opp) {
                        new_ops_this_poll.push(opp);
                    }
                }
            } // tokens

            // merge newly found ops into seen_ops and broadcast if any
            if !new_ops_this_poll.is_empty() {
                // append to seen
                for op in new_ops_this_poll.iter() {
                    seen_ops.push(op.clone());
                }

                if let Ok(json) = serde_json::to_string(&new_ops_this_poll) {
                    let _ = tx_poll.send(json);
                }
            }

            tokio::time::sleep(poll_interval).await;
        } // loop
    });

    // WebSocket route
    let ws_route = warp::path("ws")
        .and(warp::ws())
        .and_then(move |ws: warp::ws::Ws| {
            let rx = tx.subscribe();
            async move {
                Ok::<_, warp::Rejection>(ws.on_upgrade(move |socket| async move {
                    let (mut tx_ws, mut rx_ws) = socket.split();

                    // forward broadcast -> websocket
                    let mut rx_inner = rx.resubscribe();
                    let forward_task = tokio::spawn(async move {
                        while let Ok(msg) = rx_inner.recv().await {
                            if tx_ws.send(warp::ws::Message::text(msg)).await.is_err() {
                                break;
                            }
                        }
                    });

                    // read from client (discard)
                    while let Some(Ok(_msg)) = rx_ws.next().await {
                        // ignore
                    }

                    forward_task.abort();
                }))
            }
        });

    let routes = ws_route.with(warp::cors().allow_any_origin());
    println!("WebSocket server running at ws://127.0.0.1:3030/ws");
    warp::serve(routes).run(([127, 0, 0, 1], 3030)).await;

    Ok(())
}
