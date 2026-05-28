import { WebSocket } from 'ws';
import { formatUSD } from './money';
import { Listing, Auction, Bid, User } from './types';

const STDB_HOST = process.env.STDB_HOST || 'wss://stdb.example.com';
let socket: WebSocket | null = null;
let reconnectInterval = 1000; // Start with 1 second
const maxReconnectInterval = 30000; // Cap at 30 seconds

function connect() {
    if (socket) return;

    socket = new WebSocket(STDB_HOST);

    socket.onopen = () => {
        console.log('Connected to STDB');
        reconnectInterval = 1000; // Reset interval on successful connection
        subscribe();
    };

    socket.onmessage = (event) => {
        const data = JSON.parse(event.data);
        handleData(data);
    };

    socket.onclose = () => {
        console.log('Disconnected from STDB. Reconnecting...');
        setTimeout(connect, reconnectInterval);
        reconnectInterval = Math.min(reconnectInterval * 2, maxReconnectInterval);
    };

    socket.onerror = (error) => {
        console.error('WebSocket error:', error);
    };
}

function subscribe() {
    const tables = ['listings', 'auctions', 'bids', 'watchlist'];
    tables.forEach(table => {
        socket?.send(JSON.stringify({ action: 'subscribe', table }));
    });
}

function handleData(data: any) {
    switch (data.table) {
        case 'listings':
            data.rows.forEach((row: Listing) => console.log('Listing:', row));
            break;
        case 'auctions':
            data.rows.forEach((row: Auction) => console.log('Auction:', row));
            break;
        case 'bids':
            data.rows.forEach((row: Bid) => console.log('Bid:', row));
            break;
        case 'watchlist':
            data.rows.forEach((row: User) => console.log('Watchlist:', row));
            break;
        default:
            console.warn('Unknown table:', data.table);
    }
}

export function initializeStdb() {
    connect();
}

// ADR-2026-05-19-0721
// hex analyze