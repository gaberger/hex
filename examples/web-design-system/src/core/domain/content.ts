export interface ContentBlock {
  id: string;
  type: 'heading' | 'paragraph' | 'image' | 'button' | 'card' | 'list';
  content: string;
  metadata?: Record<string, unknown>;
}

export interface HeroSection {
  id: string;
  headline: string;
  subheadline: string;
  ctaLabel: string;
  ctaHref: string;
  backgroundImage?: string;
}

export interface CardItem {
  id: string;
  title: string;
  description: string;
  imageUrl?: string;
  href?: string;
  tags?: string[];
}

export interface PageLayout {
  id: string;
  sections: Section[];
}

export interface Section {
  id: string;
  type: 'hero' | 'features' | 'testimonials' | 'cta' | 'footer';
  props: HeroSection | CardItem[] | string;
}
