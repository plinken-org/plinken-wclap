// Mock catalog — free shared PLINKEN factory libraries, one row shape identical
// to what market.taluvi.com will return. Pulze = the MPC-style drum sampler
// (com.plinken.pulze); Synome = the synth (com.plinken.synome). All public +
// free, owned by the Plinken factory account, so re-pricing/scoping later is a
// field change, not a migration.

import type { Listing } from './types';

const slug = (s: string) => s.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '');

interface Extras {
  icon: string;
  story: string;
  tags: string[];
  downloads: number;
}

/** Shared defaults for every factory library row. */
function factory(
  productId: string,
  instrument: string,
  title: string,
  description: string,
  category: string,
  types: string[][],
  modes: string[],
  extras: Extras,
): Listing {
  const isPulze = instrument === 'pulze';
  return {
    id: `plinken.${instrument}.${slug(title)}`,
    kind: 'sound-bank',
    title,
    app: 'plinken',
    visibility: 'public',
    scopeOwnerType: 'org',
    scopeOwnerId: 'plinken-factory',
    productId,
    supplierId: 'plinken',
    authorName: 'PLINKEN',
    priceAmount: 0,
    priceUnit: 'plin',
    license: 'free',
    icon: extras.icon,
    description,
    story: extras.story,
    tags: extras.tags,
    downloads: extras.downloads,
    meta: {
      deviceType: 'INST',
      bankchain: [isPulze ? 'Pulze' : 'Synome', category, ''],
      types,
      modes,
      author: 'PLINKEN',
      vendor: 'PLINKEN',
    },
    // Pulze is the MPC-style drum sampler: 16 pads × 4 banks = 64 pads per kit.
    ...(isPulze ? { padLayout: { pads: 16, banks: 4 } } : {}),
  };
}

// Pulze — MPC-like drum kits.
const PULZE = 'com.plinken.pulze';
const pulze: Listing[] = [
  factory(PULZE, 'pulze', 'Neon Knock', 'Punchy neon-lit kit for modern beats.', 'Kits', [['Drums', 'Kit']], ['One-Shot'],
    { icon: '🥁', story: 'Recorded through neon-buzzing preamps for a kit that hits hard and glows in the mix.', tags: ['punchy', 'modern', 'club'], downloads: 1280 }),
  factory(PULZE, 'pulze', 'Dust Circuit', 'Dusty lo-fi drums with circuit-bent grit.', 'Lo-Fi', [['Drums', 'Lo-Fi']], ['One-Shot'],
    { icon: '📼', story: 'Sampled off dusty tape loops and mangled through circuit-bent toys.', tags: ['lofi', 'dusty', 'vintage'], downloads: 940 }),
  factory(PULZE, 'pulze', 'Metro Thump', 'Booming trap 808s and tight snares.', 'Trap', [['Drums', 'Trap']], ['One-Shot'],
    { icon: '💥', story: 'Late-night sessions chasing the deepest 808 the metro could handle.', tags: ['808', 'trap', 'sub'], downloads: 2110 }),
  factory(PULZE, 'pulze', 'Velvet Machine', 'Smooth electronic drum machine tones.', 'Electronic', [['Drums', 'Electronic']], ['One-Shot'],
    { icon: '🎛️', story: 'Classic drum machines run through velvet-smooth analog saturation.', tags: ['electronic', 'smooth', 'analog'], downloads: 760 }),
  factory(PULZE, 'pulze', 'Chrome Breaks', 'Chopped breakbeat loops with chrome sheen.', 'Breakbeat', [['Drums', 'Breakbeat']], ['Loop'],
    { icon: '🔗', story: 'Vinyl breaks chopped, reversed and chromed for the dancefloor.', tags: ['breaks', 'chopped', 'jungle'], downloads: 1495 }),
  factory(PULZE, 'pulze', 'Basement 808', 'Sub-heavy hip-hop kit from the basement.', 'Hip-Hop', [['Drums', 'Hip-Hop']], ['One-Shot'],
    { icon: '🏚️', story: 'Made in a concrete basement where the sub rattled the pipes.', tags: ['808', 'hiphop', 'sub'], downloads: 1830 }),
  factory(PULZE, 'pulze', 'Tape Sparks', 'Tape-saturated percussion with hiss and sparkle.', 'Lo-Fi', [['Drums', 'Lo-Fi']], ['One-Shot'],
    { icon: '✨', story: 'Percussion bounced to cassette until it sparkled with hiss.', tags: ['tape', 'lofi', 'percussion'], downloads: 615 }),
  factory(PULZE, 'pulze', 'Midnight Claps', 'Late-night claps and percussion one-shots.', 'Percussion', [['Drums', 'Percussion']], ['One-Shot'],
    { icon: '👏', story: 'A room full of hands clapping at midnight, layered and tuned.', tags: ['claps', 'percussion', 'night'], downloads: 1040 }),
];

// Synome — synth libraries.
const SYNOME = 'com.plinken.synome';
const synome: Listing[] = [
  factory(SYNOME, 'synome', 'Synome Drift', 'Evolving pads that drift and breathe.', 'Pad', [['Synth', 'Pad']], ['Poly'],
    { icon: '🌫️', story: 'Slow-moving pads designed to drift under dialogue and film scenes.', tags: ['pad', 'cinematic', 'evolving'], downloads: 1370 }),
  factory(SYNOME, 'synome', 'Synome Pulse', 'Rhythmic arpeggiated sequences.', 'Sequence', [['Synth', 'Sequence']], ['Arpeggiated'],
    { icon: '🔷', story: 'Tempo-synced arps built for driving, pulsing motion.', tags: ['arp', 'sequence', 'rhythmic'], downloads: 1560 }),
  factory(SYNOME, 'synome', 'Synome Velvet', 'Warm velvet keys and electric pianos.', 'Keys', [['Synth', 'Keys']], ['Poly'],
    { icon: '🎹', story: 'Soft electric keys voiced for warmth and intimacy.', tags: ['keys', 'warm', 'electric'], downloads: 880 }),
  factory(SYNOME, 'synome', 'Synome Halo', 'Ambient washes and cinematic textures.', 'Ambient', [['Synth', 'Ambient']], ['Poly'],
    { icon: '😇', story: 'Halo-bright ambient textures for scoring wide, open shots.', tags: ['ambient', 'texture', 'score'], downloads: 1210 }),
  factory(SYNOME, 'synome', 'Synome Circuit', 'Gritty mono basses with analog bite.', 'Bass', [['Synth', 'Bass']], ['Mono'],
    { icon: '⚡', story: 'Raw mono basses pushed until the circuit bit back.', tags: ['bass', 'mono', 'analog'], downloads: 1940 }),
  factory(SYNOME, 'synome', 'Synome Glass', 'Crystalline plucks and glassy bells.', 'Pluck', [['Synth', 'Pluck']], ['Poly'],
    { icon: '🔮', story: 'Bright, brittle plucks that ring like struck glass.', tags: ['pluck', 'bell', 'bright'], downloads: 990 }),
  factory(SYNOME, 'synome', 'Synome Bloom', 'Expressive leads that bloom and cut.', 'Lead', [['Synth', 'Lead']], ['Mono'],
    { icon: '🌸', story: 'Leads that open up under the mod wheel and cut through a mix.', tags: ['lead', 'expressive', 'solo'], downloads: 1420 }),
  factory(SYNOME, 'synome', 'Synome Static', 'Noise beds and FX one-shots.', 'FX', [['Synth', 'FX']], ['One-Shot'],
    { icon: '📺', story: 'Broken-signal noise beds and risers for transitions.', tags: ['fx', 'noise', 'riser'], downloads: 505 }),
];

export const MOCK_LISTINGS: Listing[] = [...pulze, ...synome];
