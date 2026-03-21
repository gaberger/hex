import { LocalContentAdapter } from '../adapters/secondary/content-adapter.js';
import { createHeroHtml } from '../adapters/primary/hero-adapter.js';
import { createCardGridHtml } from '../adapters/primary/card-adapter.js';
import type { PageLayout, HeroSection, CardItem } from '../core/domain/index.js';

export interface CompositionConfig {
  contentAdapter: LocalContentAdapter;
  onCtaClick?: () => void;
  onCardClick?: (item: CardItem) => void;
}

export class CompositionRoot {
  private config: CompositionConfig;

  constructor(config: CompositionConfig) {
    this.config = config;
  }

  async renderPage(slug: string): Promise<string> {
    const page = await this.config.contentAdapter.fetchPage(slug);
    const sections: string[] = [];

    for (const section of page.sections) {
      switch (section.type) {
        case 'hero':
          const hero = section.props as HeroSection;
          sections.push(createHeroHtml(hero));
          break;
        case 'features':
          const cards = section.props as CardItem[];
          sections.push(createCardGridHtml(cards));
          break;
      }
    }

    return sections.join('\n');
  }

  static createDefault(): CompositionRoot {
    const adapter = new LocalContentAdapter();
    
    adapter.registerHero({
      id: 'main',
      headline: 'Welcome to Our Platform',
      subheadline: 'Build faster with hexagonal architecture',
      ctaLabel: 'Get Started',
      ctaHref: '/signup',
    });

    adapter.registerCards('features', [
      { id: '1', title: 'Domain-Driven', description: 'Clean separation of concerns', href: '/domain' },
      { id: '2', title: 'Port Interfaces', description: 'Swappable adapters', href: '/ports' },
      { id: '3', title: 'Testable', description: 'Easy to mock and test', href: '/testing' },
    ]);

    return new CompositionRoot({ contentAdapter: adapter });
  }
}
