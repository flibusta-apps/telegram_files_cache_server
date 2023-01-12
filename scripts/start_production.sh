cd /app
alembic -c ./app/alembic.ini upgrade head

rm -rf prometheus
mkdir prometheus

gunicorn -k uvicorn.workers.UvicornWorker main:app --bind 0.0.0.0:8080
