// Local CI on easy-day Jenkins: every pushed branch is checked and built. See automation/README.md.
pipeline {
    agent { label 'easy-day' }
    options {
        timestamps()
        disableConcurrentBuilds()
        timeout(time: 60, unit: 'MINUTES')
        buildDiscarder(logRotator(numToKeepStr: '30'))
    }
    environment {
        CI_DROP = '/srv/data/staminator-ci'
        CI_IMAGE = 'staminator-ci:latest'
    }
    stages {
        stage('CI image') {
            steps {
                sh '''
                    set -eu
                    version=$(sed -n 's/^channel = "\\(.*\\)"/\\1/p' rust-toolchain.toml)
                    docker build --build-arg RUST_VERSION="$version" -t "$CI_IMAGE" automation/ci
                '''
            }
        }
        stage('Check and build') {
            steps {
                sh '''#!/bin/bash
                    set -euo pipefail
                    mkdir -p "$CI_DROP/cache/registry" "$CI_DROP/cache/git" "$CI_DROP/cache/target"
                    docker run --rm -u "$(id -u):$(id -g)" \
                        -e HOME=/tmp -e CARGO_TARGET_DIR=/cache/target \
                        -v "$PWD":/work \
                        -v "$CI_DROP/cache/registry":/usr/local/cargo/registry \
                        -v "$CI_DROP/cache/git":/usr/local/cargo/git \
                        -v "$CI_DROP/cache/target":/cache/target \
                        "$CI_IMAGE" bash automation/ci/run.sh 2>&1 | tee ci.log
                '''
            }
        }
    }
    post {
        always {
            script { env.CI_RESULT = currentBuild.currentResult }
            sh '''
                set -eu
                python3 automation/ci/report.py "$CI_DROP" "$(git rev-parse HEAD)" "${GIT_BRANCH:-unknown}" \
                    "$CI_RESULT" "$BUILD_URL" ci.log ci-out || true
            '''
            archiveArtifacts artifacts: 'ci-out/*.exe', allowEmptyArchive: true
        }
    }
}
