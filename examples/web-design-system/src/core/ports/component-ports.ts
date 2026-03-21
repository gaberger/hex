import type { CardItem, HeroSection, ContentBlock } from '../domain/index.js';

export interface CardPort {
  items: CardItem[];
  onItemClick?: (item: CardItem) => void;
}

export interface HeroPort {
  hero: HeroSection;
  onCtaClick?: () => void;
}

export interface ContentBlockPort {
  block: ContentBlock;
  onAction?: () => void;
}

export interface LayoutPort {
  renderHero(hero: HeroSection, onCtaClick?: () => void): Promise<string>;
  renderCardGrid(items: CardItem[], onItemClick?: (item: CardItem) => void): Promise<string>;
  renderContentBlock(block: ContentBlock, onAction?: () => void): Promise<string>;
}
