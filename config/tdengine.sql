CREATE DATABASE IF NOT EXISTS asimov PRECISION 'ms';
CREATE STABLE IF NOT EXISTS asimov.agv_visualization (ts TIMESTAMP, x DOUBLE, y DOUBLE, theta DOUBLE, vx DOUBLE, vy DOUBLE, omega DOUBLE, localization_score DOUBLE) TAGS (manufacturer VARCHAR(64), serial_number VARCHAR(64), map_id VARCHAR(64));
