// PUBLIC (open source). Mock-only market client. The HTTP client that talks to
// the real (private) market API is intentionally NOT in this public repo — the
// private app provides its own implementation of the `MarketClient` interface
// with no component changes.

import type { Listing, ListingQuery, MarketClient } from './types';
import { MOCK_LISTINGS } from './mock-data';

/** In-memory client over the mock catalog. Mirrors `GET /listings` filtering. */
export class MockMarketClient implements MarketClient {
  private readonly listings: Listing[];

  constructor(listings: Listing[] = MOCK_LISTINGS) {
    this.listings = listings;
  }

  async listListings(query: ListingQuery): Promise<Listing[]> {
    const q = query.q?.trim().toLowerCase();
    return this.listings.filter((l) => {
      if (l.app !== query.app) return false;
      if (l.productId !== query.productId) return false;
      if (query.kind && l.kind !== query.kind) return false;
      // Mock scope: only 'global' (public) content exists yet; space/mine are empty.
      if (query.scope && query.scope !== 'global' && l.visibility !== 'public') return false;
      if (query.type && !l.meta.types.some((path) => path.includes(query.type!))) return false;
      if (query.mode && !l.meta.modes.includes(query.mode)) return false;
      if (q) {
        const hay = `${l.title} ${l.description} ${l.authorName}`.toLowerCase();
        if (!hay.includes(q)) return false;
      }
      return true;
    });
  }

  async getListing(id: string): Promise<Listing | null> {
    return this.listings.find((l) => l.id === id) ?? null;
  }
}
