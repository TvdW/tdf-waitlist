@echo off
cd api
call mypy --strict .
call black --check waitlist .
call pylint waitlist dir * /s/b | findstr \.*.py$
cd ..

cd frontend
call npm run lint
call npx prettier --check .
cd ..

