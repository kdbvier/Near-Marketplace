import { JSONValue, TypedMap, near } from "@graphprotocol/graph-ts";
import { Collection } from "../generated/schema";
import {
  blockHeight,
  blockTimestamp,
  getBigIntOrZero,
  getOrCreateAccount,
  getOrCreateCollection,
  getString,
  nep297Data,
  parseNep297,
  logKey,
  recordActivity,
} from "./helpers";

/** `EVENT_STANDARD` in launchpad/src/lib.rs. */
const EVENT_STANDARD = "linear";

export function handleLaunchpadReceipt(receipt: near.ReceiptWithOutcome): void {
  const logs = receipt.outcome.logs;
  for (let i = 0; i < logs.length; i++) {
    const event = parseNep297(logs[i]);
    if (event == null) {
      continue;
    }
    if (getString(event, "standard") != EVENT_STANDARD) {
      continue;
    }
    if (getString(event, "event") != "launch") {
      continue;
    }
    const entries = nep297Data(event);
    for (let j = 0; j < entries.length; j++) {
      handleLaunch(entries[j], receipt, i);
    }
  }
}

function handleLaunch(
  data: TypedMap<string, JSONValue>,
  receipt: near.ReceiptWithOutcome,
  index: i32
): void {
  const collectionId = getString(data, "collection_id");
  if (collectionId === null) {
    return;
  }
  const timestamp = blockTimestamp(receipt);
  const collection = getOrCreateCollection(
    collectionId,
    timestamp,
    blockHeight(receipt)
  );

  const creatorId = getString(data, "creator_id");
  if (creatorId !== null) {
    collection.creator = getOrCreateAccount(creatorId).id;
  }
  collection.name = getString(data, "name");
  collection.symbol = getString(data, "symbol");
  collection.baseUri = getString(data, "base_uri");
  collection.mintCurrency = getString(data, "mint_currency");
  collection.mintPrice = getBigIntOrZero(data, "mint_price");
  collection.wlPrice = getBigIntOrZero(data, "wl_price");
  collection.royalty = getBigIntOrZero(data, "royalty");
  collection.totalSupply = getBigIntOrZero(data, "total_supply");
  collection.launched = true;
  collection.createdAt = timestamp;
  collection.createdAtBlock = blockHeight(receipt);
  collection.updatedAt = timestamp;
  collection.save();

  recordActivity(
    receipt,
    logKey(index),
    "launch",
    null,
    collection,
    creatorId,
    null,
    null
  );
}
