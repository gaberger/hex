import type { ContentPort, AssetPort } from '../../core/ports/index.js';
import type { PageLayout, HeroSection, CardItem } from '../../core/domain/index.js';

const CMS_BASE = 'https://api.example-cms.com';

export class CmsAdapter implements ContentPort {
  async fetchPage(slug: string): Promise<PageLayout> {
    const res = await fetch(`${CMS_BASE}/pages/${slug}`);
    return res.json();
  }

  async fetchHero(id: string): Promise<HeroSection> {
    const res = await fetch(`${CMS_BASE}/heroes/${id}`);
    return res.json();
  }

  async fetchCards(sectionId: string): Promise<CardItem[]> {
    const res = await fetch(`${CMS_BASE}/sections/${sectionId}/cards`);
    return res.json();
  }
}

export class LocalContentAdapter implements ContentPort {
  private pages: Map<string, PageLayout> = new Map();
  private heroes: Map<string, HeroSection> = new Map();
  private cards: Map<string, CardItem[]> = new Map();

  registerPage(layout: PageLayout): void {
    this.pages.set(layout.id, layout);
  }

  registerHero(hero: HeroSection): void {
    this.heroes.set(hero.id, hero);
  }

  registerCards(sectionId: string, items: CardItem[]): void {
    this.cards.set(sectionId, items);
  }

  async fetchPage(slug: string): Promise<PageLayout> {
    const page = this.pages.get(slug);
    if (!page) throw new Error(`Page not found: ${slug}`);
    return page;
  }

  async fetchHero(id: string): Promise<HeroSection> {
    const hero = this.heroes.get(id);
    if (!hero) throw new Error(`Hero not found: ${id}`);
    return hero;
  }

  async fetchCards(sectionId: string): Promise<CardItem[]> {
    const cards = this.cards.get(sectionId);
    if (!cards) throw new Error(`Cards not found: ${sectionId}`);
    return cards;
  }
}

export class CdnAdapter implements AssetPort {
  constructor(private baseUrl: string = '') {}

  resolveImage(path: string): string {
    return `${this.baseUrl}/images/${path}`;
  }

  resolveStylesheet(path: string): string {
    return `${this.baseUrl}/styles/${path}`;
  }
}
