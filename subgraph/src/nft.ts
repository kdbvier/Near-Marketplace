import { JSONValue, TypedMap, near } from "@graphprotocol/graph-ts";
import { Account, Nft } from "../generated/schema";
import {
  ONE,
  blockHeight,
  blockTimestamp,
  getObject,
  getOrCreateAccount,
  getOrCreateCollection,
  getOrCreateNft,
  getString,
  getStringArray,
  callKey,
  nep297Data,
  nftId,
  parseNep297,
  parseObject,
  recordActivity,
  setOwner,
  tokenLogKey,
} from "./helpers";

const NEP171 = "nep171";

/**
 * Receipts for every account matching `.factory.<launchpad>` — that is every
 * launched collection *and* every per-token vault sub-account. Vaults never
 * emit NEP-171 events and never expose `nft_mint`/`burn`, so they fall through
 * without touching the store.
 */
export function handleCollectionReceipt(
  receipt: near.ReceiptWithOutcome
): void {
  const contractId = receipt.receipt.receiverId;

  // Token metadata only exists in the `nft_mint` call arguments; the NEP-171
  // `nft_mint` event carries just the owner and the token ids.
  let mintMetadata: TypedMap<string, JSONValue> | null = null;

  const actions = receipt.receipt.actions;
  for (let i = 0; i < actions.length; i++) {
    if (actions[i].kind != near.ActionKind.FUNCTION_CALL) {
      continue;
    }
    const call = actions[i].toFunctionCall();
    const args = parseObject(call.args.toString());
    if (args == null) {
      continue;
    }
    if (call.methodName == "nft_mint") {
      mintMetadata = getObject(args, "token_metadata");
    } else if (call.methodName == "burn") {
      handleBurn(contractId, args, receipt, i);
    } else if (call.methodName == "set_mint_type") {
      handleSetMintType(contractId, args, receipt);
    } else if (call.methodName == "set_collection_owner") {
      handleSetCollectionOwner(contractId, args, receipt);
    }
  }

  const logs = receipt.outcome.logs;
  for (let i = 0; i < logs.length; i++) {
    const event = parseNep297(logs[i]);
    if (event == null || getString(event, "standard") != NEP171) {
      continue;
    }
    const name = getString(event, "event");
    const entries = nep297Data(event);
    for (let j = 0; j < entries.length; j++) {
      if (name == "nft_mint") {
        handleMint(contractId, entries[j], mintMetadata, receipt, i);
      } else if (name == "nft_transfer") {
        handleTransfer(contractId, entries[j], receipt, i);
      } else if (name == "nft_burn") {
        handleBurnEvent(contractId, entries[j], receipt, i);
      }
    }
  }
}

function handleMint(
  contractId: string,
  data: TypedMap<string, JSONValue>,
  metadata: TypedMap<string, JSONValue> | null,
  receipt: near.ReceiptWithOutcome,
  index: i32
): void {
  const ownerId = getString(data, "owner_id");
  if (ownerId === null) {
    return;
  }
  const tokenIds = getStringArray(data, "token_ids");
  const timestamp = blockTimestamp(receipt);
  const height = blockHeight(receipt);
  const collection = getOrCreateCollection(contractId, timestamp, height);

  for (let i = 0; i < tokenIds.length; i++) {
    const nft = getOrCreateNft(
      contractId,
      tokenIds[i],
      ownerId,
      timestamp,
      height
    );
    if (nft.mintedAt === null) {
      const minter = getOrCreateAccountWithMint(ownerId);
      nft.minter = minter.id;
      nft.mintedAt = timestamp;
      // `nft_mint` charges `wl_price` until `set_mint_type` opens public minting.
      nft.mintPrice = collection.isPublicMint
        ? collection.mintPrice
        : collection.wlPrice;
      collection.mintedCount = collection.mintedCount.plus(ONE);
    }
    if (metadata != null) {
      nft.title = getString(metadata, "title");
      nft.description = getString(metadata, "description");
      nft.media = getString(metadata, "media");
      nft.reference = getString(metadata, "reference");
      nft.extra = getString(metadata, "extra");
    }
    setOwner(nft, ownerId);
    nft.updatedAt = timestamp;
    nft.save();

    recordActivity(
      receipt,
      tokenLogKey(index, tokenIds[i]),
      "mint",
      nft,
      collection,
      null,
      ownerId,
      nft.mintPrice
    );
  }

  collection.updatedAt = timestamp;
  collection.save();
}

function handleTransfer(
  contractId: string,
  data: TypedMap<string, JSONValue>,
  receipt: near.ReceiptWithOutcome,
  index: i32
): void {
  const oldOwnerId = getString(data, "old_owner_id");
  const newOwnerId = getString(data, "new_owner_id");
  if (newOwnerId === null) {
    return;
  }
  const tokenIds = getStringArray(data, "token_ids");
  const timestamp = blockTimestamp(receipt);
  const height = blockHeight(receipt);

  for (let i = 0; i < tokenIds.length; i++) {
    const nft = getOrCreateNft(
      contractId,
      tokenIds[i],
      newOwnerId,
      timestamp,
      height
    );
    setOwner(nft, newOwnerId);
    nft.updatedAt = timestamp;
    nft.save();

    recordActivity(
      receipt,
      tokenLogKey(index, tokenIds[i]),
      "transfer",
      nft,
      null,
      oldOwnerId,
      newOwnerId,
      null
    );
  }
}

/**
 * The contract's `burn` deletes the token without emitting NEP-171 `nft_burn`,
 * so burns are picked up from the call itself. `handleBurnEvent` stays in place
 * in case the contract starts emitting the standard event.
 */
function handleBurn(
  contractId: string,
  args: TypedMap<string, JSONValue>,
  receipt: near.ReceiptWithOutcome,
  index: i32
): void {
  const tokenId = getString(args, "token_id");
  if (tokenId === null) {
    return;
  }
  burnToken(contractId, tokenId, receipt, callKey(index));
}

function handleBurnEvent(
  contractId: string,
  data: TypedMap<string, JSONValue>,
  receipt: near.ReceiptWithOutcome,
  index: i32
): void {
  const tokenIds = getStringArray(data, "token_ids");
  for (let i = 0; i < tokenIds.length; i++) {
    burnToken(contractId, tokenIds[i], receipt, tokenLogKey(index, tokenIds[i]));
  }
}

function burnToken(
  contractId: string,
  tokenId: string,
  receipt: near.ReceiptWithOutcome,
  key: string
): void {
  const nft = Nft.load(nftId(contractId, tokenId));
  if (nft == null || nft.burned) {
    return;
  }
  const timestamp = blockTimestamp(receipt);
  const owner = Account.load(nft.owner);
  if (owner !== null && owner.ownedCount > 0) {
    owner.ownedCount = owner.ownedCount - 1;
    owner.save();
  }
  nft.burned = true;
  nft.updatedAt = timestamp;
  nft.save();

  const collection = getOrCreateCollection(
    contractId,
    timestamp,
    blockHeight(receipt)
  );
  collection.burnedCount = collection.burnedCount.plus(ONE);
  collection.updatedAt = timestamp;
  collection.save();

  recordActivity(
    receipt,
    key,
    "burn",
    nft,
    collection,
    nft.owner,
    null,
    null
  );
}

function handleSetMintType(
  contractId: string,
  args: TypedMap<string, JSONValue>,
  receipt: near.ReceiptWithOutcome
): void {
  const timestamp = blockTimestamp(receipt);
  const collection = getOrCreateCollection(
    contractId,
    timestamp,
    blockHeight(receipt)
  );
  const isPublic = args.get("is_public_mint");
  if (isPublic != null && !isPublic.isNull()) {
    collection.isPublicMint = isPublic.toBool();
  }
  const isUnique = args.get("is_unique_mint");
  if (isUnique != null && !isUnique.isNull()) {
    collection.isUniqueMint = isUnique.toBool();
  }
  collection.updatedAt = timestamp;
  collection.save();
}

function handleSetCollectionOwner(
  contractId: string,
  args: TypedMap<string, JSONValue>,
  receipt: near.ReceiptWithOutcome
): void {
  const owner = getString(args, "owner");
  if (owner === null) {
    return;
  }
  const timestamp = blockTimestamp(receipt);
  const collection = getOrCreateCollection(
    contractId,
    timestamp,
    blockHeight(receipt)
  );
  collection.owner = owner;
  collection.updatedAt = timestamp;
  collection.save();
}

function getOrCreateAccountWithMint(accountId: string): Account {
  const account = getOrCreateAccount(accountId);
  account.mintedCount = account.mintedCount + 1;
  account.save();
  return account;
}
