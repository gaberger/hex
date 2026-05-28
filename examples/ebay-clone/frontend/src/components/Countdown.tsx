import React, { useEffect, useState } from 'react';

// ADR-2026-05-19-0721: Countdown component for auction end time
interface CountdownProps {
  endTime: Date;
  status: string; // Added to handle auction status
}

const Countdown: React.FC<CountdownProps> = ({ endTime, status }) => {
  const [timeLeft, setTimeLeft] = useState<number>(calculateTimeLeft(endTime));

  useEffect(() => {
    const timerId = setInterval(() => {
      setTimeLeft(calculateTimeLeft(endTime));
    }, 1000);

    return () => clearInterval(timerId);
  }, [endTime]);

  function calculateTimeLeft(end: Date): number {
    const difference = end.getTime() - new Date().getTime();
    return Math.max(0, Math.floor(difference / 1000));
  }

  const formatTime = (seconds: number) => {
    const minutes = Math.floor(seconds / 60);
    const secondsLeft = seconds % 60;
    return `${minutes.toString().padStart(2, '0')}:${secondsLeft.toString().padStart(2, '0')}`;
  };

  return (
    <div>
      {status === 'Active' && timeLeft > 0 ? (
        <span>Time remaining: {formatTime(timeLeft)}</span>
      ) : (
        <span>Auction ended</span>
      )}
    </div>
  );
};

export default Countdown;