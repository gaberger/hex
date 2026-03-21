import type { HeroSection } from '../../core/domain/index.js';
import type { HeroPort } from '../../core/ports/index.js';

export function createHeroHtml(hero: HeroSection): string {
  const bgStyle = hero.backgroundImage 
    ? `style="background-image: url('${hero.backgroundImage}')"` 
    : '';
  
  return `
    <header class="hero" ${bgStyle}>
      <div class="hero-content">
        <h1 class="hero-headline">${hero.headline}</h1>
        <p class="hero-subheadline">${hero.subheadline}</p>
        <a href="${hero.ctaHref}" class="hero-cta" data-action="cta-click">
          ${hero.ctaLabel}
        </a>
      </div>
    </header>
  `;
}
