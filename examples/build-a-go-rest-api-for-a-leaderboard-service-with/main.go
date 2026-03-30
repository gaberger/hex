import express from 'express';
import bodyParser from 'body-parser';

const app = express();
const PORT = 3000;

app.use(bodyParser.json());

let leaderboard: { id: number, name: string, score: number }[] = [];

// List all leaderboard entries
app.get('/leaderboard', (req, res) => {
    res.json(leaderboard);
});

// Submit a new entry to the leaderboard
app.post('/leaderboard', (req, res) => {
    const { name, score } = req.body;
    const id = leaderboard.length + 1;
    const newEntry = { id, name, score };
    leaderboard.push(newEntry);
    res.status(201).json(newEntry);
});

// Delete an entry from the leaderboard
app.delete('/leaderboard/:id', (req, res) => {
    const id = parseInt(req.params.id);
    leaderboard = leaderboard.filter(entry => entry.id !== id);
    res.sendStatus(204);
});

// Start the server
app.listen(PORT, () => {
    console.log(`Server is running on http://localhost:${PORT}`);
});