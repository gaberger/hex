import React, { useState } from 'react';
import axios from 'axios';

interface ImageUploaderProps {
  onImagesUploaded: (sha256s: string[]) => void;
}

const ImageUploader: React.FC<ImageUploaderProps> = ({ onImagesUploaded }) => {
  const [files, setFiles] = useState<File[]>([]);
  const [progresses, setProgresses] = useState<{ [key: string]: number }>({});
  const [sha256s, setSha256s] = useState<string[]>([]);

  const handleFileChange = (event: React.ChangeEvent<HTMLInputElement>) => {
    if (!event.target.files) return;
    const selectedFiles = Array.from(event.target.files);
    if (files.length + selectedFiles.length > 8) {
      alert('You can only upload up to 8 images.');
      return;
    }
    setFiles([...files, ...selectedFiles]);
  };

  const handleUpload = async () => {
    const newSha256s: string[] = [];
    for (const file of files) {
      const formData = new FormData();
      formData.append('image', file);

      try {
        const response = await axios.post('/api/v1/images', formData, {
          headers: {
            'Content-Type': 'multipart/form-data',
          },
          onUploadProgress: (progressEvent) => {
            const percentCompleted = Math.round((progressEvent.loaded * 100) / progressEvent.total);
            setProgresses(prevProgresses => ({
              ...prevProgresses,
              [file.name]: percentCompleted
            }));
          }
        });
        newSha256s.push(response.data.sha256);
      } catch (error) {
        console.error(`Failed to upload ${file.name}`, error);
      }
    }

    setFiles([]);
    setSha256s(newSha256s);
    onImagesUploaded(newSha256s);
  };

  return (
    <div>
      <input type="file" multiple accept="image/*" onChange={handleFileChange} />
      <button onClick={handleUpload}>Upload Images</button>
      {files.map((file, index) => (
        <div key={index}>
          {file.name}: {progresses[file.name] || 0}% uploaded
        </div>
      ))}
    </div>
  );
};

export default ImageUploader;
docs/specs/ebay-spec-008