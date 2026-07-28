import { BigInt, JSONValue, TypedMap, near } from "@graphprotocol/graph-ts";
import { Bid, Collection, MarketplaceStats, Nft } from "../generated/schema";
import {
  ONE,
  STATUS_NONE,
  STATUS_SALE,
  ZERO,
  bidId,
  blockHeight,
  blockTimestamp,
  getBigInt,
  getBigIntOrZero,
  getBool,
  getObject,
  getOrCreateAccount,
  getOrCreateCollection,
  getOrCreateNft,
  getOrCreateStats,
  getString,
  nftId,
  onListed,
  onUnlisted,
  parseObject,
  logKey,
  recordActivity,
  setOwner,
} from "./helpers";

/** `FIVE_MINUTES` from the marketplace contract, in nanoseconds. */
const FIVE_MINUTES = BigInt.fromString("300000000000");

/**
 * graph-node only surfaces receipts whose outcome succeeded (`ExecutionOutcome`
 * carries a `SuccessStatus`), so both the emitted logs and the function-call
 * actions on a receipt reflect committed state.
 */
export function handleMarketplaceReceipt(
  receipt: near.ReceiptWithOutcome
): void {
  const actions = receipt.receipt.actions;
  for (let i = 0; i < actions.length; i++) {
    if (actions[i].kind == near.ActionKind.FUNCTION_CALL) {
      handleFunctionCall(actions[i].toFunctionCall(), receipt, i);
    }
  }

  const logs = receipt.outcome.logs;
  for (let i = 0; i < logs.length; i++) {
    handleLog(logs[i], i, receipt);
  }
}

/**
 * `internal_process_purchase` deletes the market data *before* calling
 * `nft_transfer_payout` and never restores it, and it emits no log on that
 * path. If the transfer fails the token is silently unlisted on chain with only
 * a refund. Reacting to the `buy` / `accept_bid` calls themselves keeps the
 * listing state in sync in both the success and the failure case.
 */
function handleFunctionCall(
  call: near.FunctionCallAction,
  receipt: near.ReceiptWithOutcome,
  index: i32
): void {
  if (call.methodName != "buy" && call.methodName != "accept_bid") {
    return;
  }
  const args = parseObject(call.args.toString());
  if (args == null) {
    return;
  }
  // Separate null checks: the AssemblyScript compiler only narrows
  // `string | null` locals through a single-condition guard.
  const contractId = getString(args, "nft_contract_id");
  if (contractId === null) {
    return;
  }
  const tokenId = getString(args, "token_id");
  if (tokenId === null) {
    return;
  }
  const nft = Nft.load(nftId(contractId, tokenId));
  if (nft == null) {
    return;
  }
  const collection = getOrCreateCollection(
    contractId,
    blockTimestamp(receipt),
    blockHeight(receipt)
  );
  unlist(nft, collection, getOrCreateStats(), blockTimestamp(receipt));
  nft.save();
  collection.save();
}

function handleLog(
  raw: string,
  index: i32,
  receipt: near.ReceiptWithOutcome
): void {
  // The marketplace logs bare JSON (`{"type": ..., "params": {...}}`), not the
  // NEP-297 `EVENT_JSON:` envelope the launchpad and the NFT contract use.
  const event = parseObject(raw);
  if (event == null) {
    return;
  }
  const type = getString(event, "type");
  const params = getObject(event, "params");
  if (type === null || params == null) {
    return;
  }

  if (type == "add_market_data") {
    handleAddMarketData(params, receipt, index);
  } else if (type == "delete_market_data") {
    handleDeleteMarketData(params, receipt, index);
  } else if (type == "resolve_purchase") {
    handleResolvePurchase(params, receipt, index);
  } else if (type == "add_bid") {
    handleAddBid(params, receipt, index);
  } else if (type == "cancel_bid") {
    handleCancelBid(params, receipt, index);
  } else if (type == "extend_auction") {
    handleExtendAuction(params, receipt, index);
  }
}

function handleAddMarketData(
  params: TypedMap<string, JSONValue>,
  receipt: near.ReceiptWithOutcome,
  index: i32
): void {
  const contractId = getString(params, "nft_contract_id");
  if (contractId === null) {
    return;
  }
  const tokenId = getString(params, "token_id");
  if (tokenId === null) {
    return;
  }
  const ownerId = getString(params, "owner_id");
  if (ownerId === null) {
    return;
  }

  const timestamp = blockTimestamp(receipt);
  const height = blockHeight(receipt);
  const nft = getOrCreateNft(contractId, tokenId, ownerId, timestamp, height);
  const collection = getOrCreateCollection(contractId, timestamp, height);
  const stats = getOrCreateStats();
  const price = getBigIntOrZero(params, "price");

  // `nft_on_approve` can fire again for an already listed token (re-price).
  if (nft.marketStatus != STATUS_SALE) {
    collection.listedCount = collection.listedCount.plus(ONE);
    stats.listedCount = stats.listedCount.plus(ONE);
  }

  nft.marketStatus = STATUS_SALE;
  nft.price = price;
  nft.endPrice = getBigInt(params, "end_price");
  nft.isAuction = getBool(params, "is_auction");
  nft.seller = ownerId;
  nft.approvalId = getBigInt(params, "approval_id");
  nft.startedAt = getBigInt(params, "started_at");
  nft.endedAt = getBigInt(params, "ended_at");
  nft.listedAt = timestamp;
  nft.updatedAt = timestamp;

  onListed(collection, price);
  collection.updatedAt = timestamp;

  nft.save();
  collection.save();
  stats.save();

  recordActivity(receipt, logKey(index), "list", nft, collection, ownerId, null, price);
}

function handleDeleteMarketData(
  params: TypedMap<string, JSONValue>,
  receipt: near.ReceiptWithOutcome,
  index: i32
): void {
  const contractId = getString(params, "nft_contract_id");
  if (contractId === null) {
    return;
  }
  const tokenId = getString(params, "token_id");
  if (tokenId === null) {
    return;
  }
  const nft = Nft.load(nftId(contractId, tokenId));
  if (nft == null) {
    return;
  }
  const timestamp = blockTimestamp(receipt);
  const collection = getOrCreateCollection(
    contractId,
    timestamp,
    blockHeight(receipt)
  );
  const stats = getOrCreateStats();

  unlist(nft, collection, stats, timestamp);
  nft.save();
  collection.save();
  stats.save();

  recordActivity(
    receipt,
    logKey(index),
    "unlist",
    nft,
    collection,
    getString(params, "owner_id"),
    null,
    null
  );
}

function handleResolvePurchase(
  params: TypedMap<string, JSONValue>,
  receipt: near.ReceiptWithOutcome,
  index: i32
): void {
  const contractId = getString(params, "nft_contract_id");
  if (contractId === null) {
    return;
  }
  const tokenId = getString(params, "token_id");
  if (tokenId === null) {
    return;
  }
  const sellerId = getString(params, "owner_id");
  if (sellerId === null) {
    return;
  }
  const buyerId = getString(params, "buyer_id");
  if (buyerId === null) {
    return;
  }

  const timestamp = blockTimestamp(receipt);
  const height = blockHeight(receipt);
  const price = getBigIntOrZero(params, "price");
  const nft = getOrCreateNft(contractId, tokenId, sellerId, timestamp, height);
  const collection = getOrCreateCollection(contractId, timestamp, height);
  const stats = getOrCreateStats();

  // Normally already unlisted by the `buy` / `accept_bid` action above.
  unlist(nft, collection, stats, timestamp);
  setOwner(nft, buyerId);
  nft.lastSalePrice = price;
  nft.lastSaleAt = timestamp;
  nft.saleCount = nft.saleCount.plus(ONE);
  nft.updatedAt = timestamp;
  nft.save();

  collection.volume = collection.volume.plus(price);
  collection.totalSales = collection.totalSales.plus(ONE);
  collection.updatedAt = timestamp;
  collection.save();

  stats.totalVolume = stats.totalVolume.plus(price);
  stats.totalSales = stats.totalSales.plus(ONE);
  stats.save();

  // Gross amounts: the marketplace fee and the royalty split are deducted
  // inside `resolve_purchase` and are not part of the log.
  const buyer = getOrCreateAccount(buyerId);
  buyer.totalSpent = buyer.totalSpent.plus(price);
  buyer.save();

  const seller = getOrCreateAccount(sellerId);
  seller.totalEarned = seller.totalEarned.plus(price);
  seller.save();

  recordActivity(
    receipt,
    logKey(index),
    "sale",
    nft,
    collection,
    sellerId,
    buyerId,
    price
  );
}

function handleAddBid(
  params: TypedMap<string, JSONValue>,
  receipt: near.ReceiptWithOutcome,
  index: i32
): void {
  const contractId = getString(params, "nft_contract_id");
  if (contractId === null) {
    return;
  }
  const tokenId = getString(params, "token_id");
  if (tokenId === null) {
    return;
  }
  const bidderId = getString(params, "bidder_id");
  if (bidderId === null) {
    return;
  }
  const nft = Nft.load(nftId(contractId, tokenId));
  if (nft == null) {
    return;
  }

  const timestamp = blockTimestamp(receipt);
  const amount = getBigIntOrZero(params, "amount");
  const account = getOrCreateAccount(bidderId);

  const id = bidId(nft.id, bidderId);
  let bid = Bid.load(id);
  if (bid == null) {
    bid = new Bid(id);
    bid.nft = nft.id;
    bid.bidder = account.id;
    bid.createdAt = timestamp;
  }
  const wasActive = bid.active;
  bid.amount = amount;
  bid.active = true;
  bid.updatedAt = timestamp;
  bid.save();

  if (!wasActive) {
    const bidders = nft.activeBidders;
    bidders.push(bidderId);
    nft.activeBidders = bidders;
    nft.bidCount = nft.bidCount + 1;
  }

  // The contract enforces each bid to beat the previous one, so the newest bid
  // is always the top bid.
  nft.topBid = bid.id;

  // `add_bid` logs `ended_at` as the *bid* timestamp, while the contract stores
  // `ended_at = block_timestamp + FIVE_MINUTES`. Mirror the stored value.
  const bidTime = getBigInt(params, "ended_at");
  nft.endedAt = (bidTime === null ? timestamp : bidTime).plus(FIVE_MINUTES);
  nft.updatedAt = timestamp;
  nft.save();

  recordActivity(receipt, logKey(index), "bid", nft, null, bidderId, null, amount);
}

function handleCancelBid(
  params: TypedMap<string, JSONValue>,
  receipt: near.ReceiptWithOutcome,
  index: i32
): void {
  const contractId = getString(params, "nft_contract_id");
  if (contractId === null) {
    return;
  }
  const tokenId = getString(params, "token_id");
  if (tokenId === null) {
    return;
  }
  const bidderId = getString(params, "bidder_id");
  if (bidderId === null) {
    return;
  }
  const nft = Nft.load(nftId(contractId, tokenId));
  if (nft == null) {
    return;
  }
  const timestamp = blockTimestamp(receipt);
  const bid = Bid.load(bidId(nft.id, bidderId));
  if (bid != null && bid.active) {
    bid.active = false;
    bid.updatedAt = timestamp;
    bid.save();
    removeBidder(nft, bidderId);
  }
  nft.updatedAt = timestamp;
  nft.save();

  recordActivity(receipt, logKey(index), "cancel_bid", nft, null, bidderId, null, null);
}

function handleExtendAuction(
  params: TypedMap<string, JSONValue>,
  receipt: near.ReceiptWithOutcome,
  index: i32
): void {
  const contractId = getString(params, "nft_contract_id");
  if (contractId === null) {
    return;
  }
  const tokenId = getString(params, "token_id");
  if (tokenId === null) {
    return;
  }
  const nft = Nft.load(nftId(contractId, tokenId));
  if (nft == null) {
    return;
  }
  const endedAt = getBigInt(params, "ended_at");
  if (endedAt !== null) {
    nft.endedAt = endedAt;
  }
  nft.updatedAt = blockTimestamp(receipt);
  nft.save();

  recordActivity(
    receipt,
    logKey(index),
    "extend_auction",
    nft,
    null,
    null,
    null,
    null
  );
}

// ---------------------------------------------------------------------------

function unlist(
  nft: Nft,
  collection: Collection,
  stats: MarketplaceStats,
  timestamp: BigInt
): void {
  if (nft.marketStatus != STATUS_SALE) {
    return;
  }
  const price = nft.price;

  nft.marketStatus = STATUS_NONE;
  nft.price = ZERO;
  nft.endPrice = null;
  nft.isAuction = false;
  nft.seller = null;
  nft.approvalId = null;
  nft.startedAt = null;
  nft.endedAt = null;
  nft.topBid = null;
  nft.updatedAt = timestamp;

  // Delisting refunds every outstanding bid on chain.
  const bidders = nft.activeBidders;
  for (let i = 0; i < bidders.length; i++) {
    const bid = Bid.load(bidId(nft.id, bidders[i]));
    if (bid != null && bid.active) {
      bid.active = false;
      bid.updatedAt = timestamp;
      bid.save();
    }
  }
  nft.activeBidders = new Array<string>();
  nft.bidCount = 0;

  collection.listedCount = collection.listedCount.minus(ONE);
  if (collection.listedCount.lt(ZERO)) {
    collection.listedCount = ZERO;
  }
  collection.updatedAt = timestamp;
  onUnlisted(collection, price);

  stats.listedCount = stats.listedCount.minus(ONE);
  if (stats.listedCount.lt(ZERO)) {
    stats.listedCount = ZERO;
  }
  stats.save();
}

function removeBidder(nft: Nft, bidderId: string): void {
  const bidders = nft.activeBidders;
  const kept = new Array<string>();
  for (let i = 0; i < bidders.length; i++) {
    if (bidders[i] != bidderId) {
      kept.push(bidders[i]);
    }
  }
  nft.activeBidders = kept;
  nft.bidCount = kept.length;

  // Re-seed the top bid from whatever is left.
  let top: Bid | null = null;
  for (let i = 0; i < kept.length; i++) {
    const bid = Bid.load(bidId(nft.id, kept[i]));
    if (bid == null || !bid.active) {
      continue;
    }
    if (top === null || bid.amount.gt(top.amount)) {
      top = bid;
    }
  }
  nft.topBid = top === null ? null : top.id;
}
