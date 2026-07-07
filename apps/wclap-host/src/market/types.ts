// PUBLIC (open source). Client-side contract for the Taluvi `market` service.
// This file only describes the shape the browser consumes — pure data types,
// no server logic and no credentials. The actual market API/server (D1 schema,
// auth, payouts) is PRIVATE and lives in taluvi-mono/market — intentionally not
// part of this repo.

/** NKS NISI-derived metadata — the queryable facets for a sound listing. */
export interface NksMeta {
  deviceType: string; // "INST" | "FX" | "MIDI"
  bankchain: string[]; // ≤3 levels
  types: string[][]; // nested category paths
  modes: string[];
  author: string;
  vendor: string; // "PLINKEN" for factory content
}

export interface Listing {
  id: string;
  kind: 'sound-bank' | 'sound-preset' | 'foley' | 'sample-pack' | string;
  title: string;
  app: string;
  visibility: 'public' | 'space' | 'user';
  scopeOwnerType: string;
  scopeOwnerId: string;
  /** NKS PLID plugin identity — the only input that differs per instrument. */
  productId: string;
  supplierId: string;
  authorName: string;
  authorAvatarRef?: string;
  priceAmount: number;
  priceUnit: 'plin' | 'usd_micros';
  license: 'free' | 'paid' | 'subscription-included';
  previewRef?: string;
  artifactRef?: string;
  icon?: string;
  artworkRef?: string;
  description: string;
  story?: string;
  tags: string[];
  downloads: number;
  meta: NksMeta;
  /** MPC-style pad kits (Pulze): 16 pads × 4 banks = 64. Absent for synth libs. */
  padLayout?: { pads: number; banks: number };
}

export type Scope = 'global' | 'space' | 'mine';

export interface ListingQuery {
  app: string;
  productId: string;
  kind?: string;
  scope?: Scope;
  q?: string;
  type?: string;
  mode?: string;
}

/** The one interface the browser depends on. Mock now, HTTP later. */
export interface MarketClient {
  listListings(query: ListingQuery): Promise<Listing[]>;
  getListing(id: string): Promise<Listing | null>;
}
