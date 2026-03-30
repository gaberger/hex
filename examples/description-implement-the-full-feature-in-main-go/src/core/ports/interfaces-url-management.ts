import { URL } from '../domain/value-objects/url.js';
import { URLCollection } from '../domain/entities/url-collection.js';

export interface URLRepository {
  save(url: URL): Promise<void>;
  findById(id: string): Promise<URL | null>;
  findAll(): Promise<URL[]>;
  delete(id: string): Promise<boolean>;
}

export interface URLCollectionRepository {
  save(collection: URLCollection): Promise<void>;
  findById(id: string): Promise<URLCollection | null>;
  findAll(): Promise<URLCollection[]>;
  delete(id: string): Promise<boolean>;
  addURLToCollection(collectionId: string, url: URL): Promise<void>;
  removeURLFromCollection(collectionId: string, urlId: string): Promise<void>;
}

export interface URLManagementService {
  createURL(url: string, title?: string, tags?: string[]): Promise<URL>;
  getURL(id: string): Promise<URL | null>;
  getAllURLs(): Promise<URL[]>;
  updateURL(id: string, updates: Partial<{ url: string; title: string; tags: string[] }>): Promise<URL>;
  deleteURL(id: string): Promise<boolean>;
  
  createCollection(name: string, description?: string): Promise<URLCollection>;
  getCollection(id: string): Promise<URLCollection | null>;
  getAllCollections(): Promise<URLCollection[]>;
  updateCollection(id: string, updates: Partial<{ name: string; description: string }>): Promise<URLCollection>;
  deleteCollection(id: string): Promise<boolean>;
  
  addURLToCollection(collectionId: string, urlId: string): Promise<void>;
  removeURLFromCollection(collectionId: string, urlId: string): Promise<void>;
}