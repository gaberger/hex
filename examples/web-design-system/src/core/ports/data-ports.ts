import type { PageLayout, HeroSection, CardItem } from '../domain/index.js';

export interface ContentPort {
  fetchPage(slug: string): Promise<PageLayout>;
  fetchHero(id: string): Promise<HeroSection>;
  fetchCards(sectionId: string): Promise<CardItem[]>;
}

export interface AssetPort {
  resolveImage(path: string): string;
  resolveStylesheet(path: string): string;
}
