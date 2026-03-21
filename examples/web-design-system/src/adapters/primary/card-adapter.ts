import type { CardItem } from '../../core/domain/index.js';
import type { CardPort } from '../../core/ports/index.js';

export function createCardHtml(item: CardItem): string {
  return `
    <article class="card" data-id="${item.id}">
      ${item.imageUrl ? `<img src="${item.imageUrl}" alt="${item.title}" class="card-image" loading="lazy">` : ''}
      <div class="card-body">
        <h3 class="card-title">${item.title}</h3>
        <p class="card-description">${item.description}</p>
        ${item.tags ? `<div class="card-tags">${item.tags.map(t => `<span class="tag">${t}</span>`).join('')}</div>` : ''}
      </div>
    </article>
  `;
}

export function createCardGridHtml(items: CardItem[]): string {
  return `
    <section class="card-grid" aria-label="Feature cards">
      ${items.map(createCardHtml).join('')}
    </section>
  `;
}

export function createCardGridPort(items: CardItem[], onClick?: (item: CardItem) => void): string {
  return createCardGridHtml(items);
}
