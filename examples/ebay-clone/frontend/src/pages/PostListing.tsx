import React, { useState } from 'react';
import ImageUploader from '../components/ImageUploader';
import PriceInput from '../components/PriceInput';
import DurationPicker from '../components/DurationPicker';

const PostListing = () => {
    const [title, setTitle] = useState('');
    const [description, setDescription] = useState('');
    const [startingPrice, setStartingPrice] = useState(0);
    const [duration, setDuration] = useState('60s');
    const [imageHashes, setImageHashes] = useState<string[]>([]);
    const [errors, setErrors] = useState<{[key: string]: string}>({});

    const validateForm = () => {
        let valid = true;
        const newErrors: {[key: string]: string} = {};
        
        if (!title) {
            newErrors.title = 'Title is required';
            valid = false;
        }
        if (!description) {
            newErrors.description = 'Description is required';
            valid = false;
        }
        if (startingPrice <= 0) {
            newErrors.startingPrice = 'Starting price must be greater than zero';
            valid = false;
        }
        if (imageHashes.length === 0 || imageHashes.length > 8) {
            newErrors.images = `You must upload between 1 and 8 images, currently you have ${imageHashes.length}`;
            valid = false;
        }

        setErrors(newErrors);
        return valid;
    };

    const handleSubmit = async (e: React.FormEvent) => {
        e.preventDefault();
        if (!validateForm()) return;

        try {
            const listingResponse = await fetch('/api/v1/listings', {
                method: 'POST',
                headers: {
                    'Content-Type': 'application/json',
                },
                body: JSON.stringify({
                    title,
                    description,
                    startingPrice,
                    duration,
                    imageHashes
                })
            });

            if (!listingResponse.ok) {
                throw new Error('Failed to create listing');
            }

            alert('Listing created successfully!');
        } catch (error) {
            console.error(error);
            alert('An error occurred while creating your listing. Please try again.');
        }
    };

    return (
        <div>
            <h1>Post a New Listing</h1>
            <form onSubmit={handleSubmit}>
                <label>
                    Title:
                    <input type="text" value={title} onChange={(e) => setTitle(e.target.value)} />
                    {errors.title && <span>{errors.title}</span>}
                </label>
                <br />
                <label>
                    Description:
                    <textarea value={description} onChange={(e) => setDescription(e.target.value)} />
                    {errors.description && <span>{errors.description}</span>}
                </label>
                <br />
                <PriceInput price={startingPrice} setPrice={setStartingPrice} errors={errors.startingPrice} />
                <DurationPicker duration={duration} setDuration={setDuration} />
                <ImageUploader setImageHashes={setImageHashes} imageCount={imageHashes.length} errors={errors.images} />
                <button type="submit">Submit</button>
            </form>
        </div>
    );
};

export default PostListing;
docs/specs/ebay-spec-006