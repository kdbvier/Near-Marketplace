import {
  BigInt,
  JSONValue,
  JSONValueKind,
  TypedMap,
  json,
  near,
} from "@graphprotocol/graph-ts";
import {
  Account,
  Activity,
  Collection,
  MarketplaceStats,
  Nft,
} from "../generated/schema";

/** Same key separator the marketplace contract uses for `ContractAndTokenId`. */
export const DELIMETER = "||";
export const EVENT_JSON_PREFIX = "EVENT_JSON:";
export const STATS_ID = "marketplace";

export const ZERO = BigInt.zero();
export const ONE = BigInt.fromI32(1);

export const STATUS_NONE = "none";
export const STATUS_SALE = "sale";

export function nftId(contractId: string, tokenId: string): string {
  return contractId + DELIMETER + tokenId;
}

export function bidId(nft: string, bidder: string): string {
  return nft + DELIMETER + bidder;
}

// ---------------------------------------------------------------------------
// JSON helpers.
//
// The contracts serialize `U128`/`U64` as JSON strings but plain `u64`/`u16`
// as JSON numbers, so every accessor has to tolerate both.
// ---------------------------------------------------------------------------

export function parseObject(raw: string): TypedMap<string, JSONValue> | null {
  const parsed = json.try_fromString(raw);
  if (parsed.isError) {
    return null;
  }
  const value = parsed.value;
  if (value.kind != JSONValueKind.OBJECT) {
    return null;
  }
  return value.toObject();
}

export function getObject(
  obj: TypedMap<string, JSONValue>,
  key: string
): TypedMap<string, JSONValue> | null {
  const value = obj.get(key);
  if (value == null || value.isNull() || value.kind != JSONValueKind.OBJECT) {
    return null;
  }
  return value.toObject();
}

export function getString(
  obj: TypedMap<string, JSONValue>,
  key: string
): string | null {
  const value = obj.get(key);
  if (value == null || value.isNull()) {
    return null;
  }
  if (value.kind == JSONValueKind.STRING) {
    return value.toString();
  }
  if (value.kind == JSONValueKind.NUMBER) {
    return value.toBigInt().toString();
  }
  return null;
}

export function getBigInt(
  obj: TypedMap<string, JSONValue>,
  key: string
): BigInt | null {
  const value = obj.get(key);
  if (value == null || value.isNull()) {
    return null;
  }
  if (value.kind == JSONValueKind.STRING) {
    const raw = value.toString();
    if (raw.length == 0) {
      return null;
    }
    return BigInt.fromString(raw);
  }
  if (value.kind == JSONValueKind.NUMBER) {
    return value.toBigInt();
  }
  return null;
}

export function getBigIntOrZero(
  obj: TypedMap<string, JSONValue>,
  key: string
): BigInt {
  const value = getBigInt(obj, key);
  return value === null ? ZERO : value;
}

export function getBool(obj: TypedMap<string, JSONValue>, key: string): boolean {
  const value = obj.get(key);
  if (value == null || value.isNull() || value.kind != JSONValueKind.BOOL) {
    return false;
  }
  return value.toBool();
}

export function getStringArray(
  obj: TypedMap<string, JSONValue>,
  key: string
): string[] {
  const out = new Array<string>();
  const value = obj.get(key);
  if (value == null || value.isNull() || value.kind != JSONValueKind.ARRAY) {
    return out;
  }
  const items = value.toArray();
  for (let i = 0; i < items.length; i++) {
    if (items[i].kind == JSONValueKind.STRING) {
      out.push(items[i].toString());
    }
  }
  return out;
}

/**
 * NEP-297 events are logged as `EVENT_JSON:{...}`. Returns the payload object
 * only when the log actually carries that prefix.
 */
export function parseNep297(raw: string): TypedMap<string, JSONValue> | null {
  if (!raw.startsWith(EVENT_JSON_PREFIX)) {
    return null;
  }
  return parseObject(raw.substr(EVENT_JSON_PREFIX.length));
}

/** `data` of a NEP-297 event is always an array; most events carry one entry. */
export function nep297Data(
  event: TypedMap<string, JSONValue>
): Array<TypedMap<string, JSONValue>> {
  const out = new Array<TypedMap<string, JSONValue>>();
  const data = event.get("data");
  if (data == null || data.isNull() || data.kind != JSONValueKind.ARRAY) {
    return out;
  }
  const items = data.toArray();
  for (let i = 0; i < items.length; i++) {
    if (items[i].kind == JSONValueKind.OBJECT) {
      out.push(items[i].toObject());
    }
  }
  return out;
}

// ---------------------------------------------------------------------------
// Entity loaders
// ---------------------------------------------------------------------------

export function getOrCreateStats(): MarketplaceStats {
  let stats = MarketplaceStats.load(STATS_ID);
  if (stats == null) {
    stats = new MarketplaceStats(STATS_ID);
    stats.totalVolume = ZERO;
    stats.totalSales = ZERO;
    stats.listedCount = ZERO;
    stats.save();
  }
  return stats;
}

export function getOrCreateAccount(accountId: string): Account {
  let account = Account.load(accountId);
  if (account == null) {
    account = new Account(accountId);
    account.ownedCount = 0;
    account.mintedCount = 0;
    account.totalSpent = ZERO;
    account.totalEarned = ZERO;
    account.save();
  }
  return account;
}

/**
 * Collections normally come from a launchpad `launch` event. A listing can
 * still reference a collection this subgraph never saw launched (an externally
 * deployed contract that the marketplace owner approved), so create a stub.
 */
export function getOrCreateCollection(
  contractId: string,
  timestamp: BigInt,
  blockHeight: BigInt
): Collection {
  let collection = Collection.load(contractId);
  if (collection == null) {
    collection = new Collection(contractId);
    collection.isPublicMint = false;
    collection.isUniqueMint = true;
    collection.mintPrice = ZERO;
    collection.wlPrice = ZERO;
    collection.royalty = ZERO;
    collection.totalSupply = ZERO;
    collection.mintedCount = ZERO;
    collection.burnedCount = ZERO;
    collection.listedCount = ZERO;
    collection.volume = ZERO;
    collection.totalSales = ZERO;
    collection.launched = false;
    collection.createdAt = timestamp;
    collection.createdAtBlock = blockHeight;
    collection.updatedAt = timestamp;
    collection.save();
  }
  return collection;
}

export function getOrCreateNft(
  contractId: string,
  tokenId: string,
  ownerId: string,
  timestamp: BigInt,
  blockHeight: BigInt
): Nft {
  const id = nftId(contractId, tokenId);
  let nft = Nft.load(id);
  if (nft == null) {
    nft = new Nft(id);
    nft.collection = getOrCreateCollection(contractId, timestamp, blockHeight).id;
    nft.tokenId = tokenId;
    nft.owner = getOrCreateAccount(ownerId).id;
    nft.marketStatus = STATUS_NONE;
    nft.price = ZERO;
    nft.isAuction = false;
    nft.bidCount = 0;
    nft.activeBidders = new Array<string>();
    nft.saleCount = ZERO;
    nft.burned = false;
    nft.updatedAt = timestamp;
    nft.save();

    const owner = getOrCreateAccount(ownerId);
    owner.ownedCount = owner.ownedCount + 1;
    owner.save();
  }
  return nft;
}

/**
 * Idempotent owner change. A marketplace purchase produces both a
 * `resolve_purchase` log (Marketplace data source) and an `nft_transfer` NEP-171
 * event (Collections data source); whichever lands first wins and the second
 * one is a no-op, so `ownedCount` stays correct.
 */
export function setOwner(nft: Nft, newOwnerId: string): void {
  if (nft.owner == newOwnerId) {
    return;
  }
  const previous = Account.load(nft.owner);
  if (previous != null && previous.ownedCount > 0) {
    previous.ownedCount = previous.ownedCount - 1;
    previous.save();
  }
  const next = getOrCreateAccount(newOwnerId);
  next.ownedCount = next.ownedCount + 1;
  next.save();
  nft.owner = newOwnerId;
}

// ---------------------------------------------------------------------------
// Floor price
//
// `Nft.price` is 0 for unlisted tokens, so the floor has to be recomputed from
// the listed set. `Collection.nfts` is a derived field and cannot be loaded
// from a mapping, so the floor is maintained incrementally:
//   - a new/changed listing can only lower it
//   - an unlist/sale at the floor invalidates it (set to null and let the next
//     listing re-seed it)
// ---------------------------------------------------------------------------

export function onListed(collection: Collection, price: BigInt): void {
  const floor = collection.floorPrice;
  if (floor === null || price.lt(floor)) {
    collection.floorPrice = price;
  }
}

export function onUnlisted(collection: Collection, price: BigInt): void {
  const floor = collection.floorPrice;
  if (collection.listedCount.le(ZERO)) {
    collection.floorPrice = null;
  } else if (floor !== null && price.le(floor)) {
    // The cheapest listing is gone; the next listing re-seeds the floor.
    collection.floorPrice = null;
  }
}

// ---------------------------------------------------------------------------
// Activity
// ---------------------------------------------------------------------------

/** Activity key for a log-derived event. */
export function logKey(index: i32): string {
  return "log" + index.toString();
}

/** Activity key for a log-derived event that fans out over token ids. */
export function tokenLogKey(index: i32, tokenId: string): string {
  return "log" + index.toString() + "-" + tokenId;
}

/** Activity key for an event derived from a function-call action. */
export function callKey(index: i32): string {
  return "call" + index.toString();
}

/**
 * `key` must be unique within a receipt: a single log can produce one Activity
 * per token id, and action-derived activities share the index space with
 * log-derived ones.
 */
export function recordActivity(
  receipt: near.ReceiptWithOutcome,
  key: string,
  type: string,
  nft: Nft | null,
  collection: Collection | null,
  from: string | null,
  to: string | null,
  price: BigInt | null
): void {
  const receiptId = receipt.receipt.id.toBase58();
  const activity = new Activity(receiptId + "-" + key);
  activity.type = type;
  if (nft !== null) {
    activity.nft = nft.id;
    activity.collection = nft.collection;
  }
  if (collection !== null) {
    activity.collection = collection.id;
  }
  activity.from = from;
  activity.to = to;
  activity.price = price;
  activity.timestamp = blockTimestamp(receipt);
  activity.blockHeight = blockHeight(receipt);
  activity.receiptId = receiptId;
  activity.save();
}

export function blockTimestamp(receipt: near.ReceiptWithOutcome): BigInt {
  return BigInt.fromU64(receipt.block.header.timestampNanosec);
}

export function blockHeight(receipt: near.ReceiptWithOutcome): BigInt {
  return BigInt.fromU64(receipt.block.header.height);
}
