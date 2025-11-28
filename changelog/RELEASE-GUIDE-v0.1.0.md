# 🚀 Release Guide for v$NEW_VERSION

## 📋 How to Upgrade the Release Branch

### 🔄 Sync with Latest Changes
```bash
# Switch to the release branch
git checkout $BRANCH_NAME

# Pull latest changes from remote
git pull origin $BRANCH_NAME

# Merge latest changes from main (if needed)
git merge origin/main

# Push updated branch
git push origin $BRANCH_NAME
```

### 🔧 Manual Version Updates (if needed)
```bash
# Update package.json version manually
npm version $NEW_VERSION --no-git-tag-version

# Update all Cargo.toml files
find . -name "Cargo.toml" -not -path "./target/*" -exec sed -i 's/^version = ".*"/version = "$NEW_VERSION"/' {} \;

# Update frontend package.json files
find apps libs -name "package.json" -exec jq '.version = "$NEW_VERSION"' {} \; > tmp && mv tmp {}

# Commit version changes
git add .
git commit -m "chore: manual version bump to v$NEW_VERSION"
git push origin $BRANCH_NAME
```

## ✍️ How to Sign All Commits in the Branch

### 🔑 Setup GPG Signing (First Time)
```bash
# Generate a new GPG key (if you don't have one)
gpg --full-generate-key

# List your GPG keys and copy the key ID
gpg --list-secret-keys --keyid-format=long

# Configure git to use your GPG key
git config --global user.signingkey YOUR_KEY_ID
git config --global commit.gpgsign true

# Export your public key and add it to GitHub
gpg --armor --export YOUR_KEY_ID
```

### 📝 Sign Commits Retroactively
```bash
# Switch to release branch
git checkout $BRANCH_NAME

# Interactive rebase to sign all commits since main
git rebase --exec 'git commit --amend --no-edit -S' origin/main

# Alternative: Sign specific range of commits
git rebase -i --exec 'git commit --amend --no-edit -S' HEAD~N  # N = number of commits

# Force push the signed commits (⚠️ Use with caution)
git push --force-with-lease origin $BRANCH_NAME
```

### 🔒 Sign Individual New Commits
```bash
# Enable signing for current repository
git config commit.gpgsign true

# Make a signed commit
git commit -S -m "feat: your commit message"

# Verify commit is signed
git log --show-signature -1
```

## 🛠️ Troubleshooting

### 🔍 Common Issues

#### GPG Key Not Found
```bash
# Check if GPG is working
gpg --version

# List available keys
gpg --list-keys

# Test GPG signing
echo "test" | gpg --clearsign
```

#### Merge Conflicts During Upgrade
```bash
# Abort current merge
git merge --abort

# Try merge with strategy
git merge -X ours origin/main  # Prefer release branch changes
# OR
git merge -X theirs origin/main  # Prefer main branch changes

# Manual conflict resolution
git mergetool
git commit -m "resolve: merge conflicts with main"
```

#### Force Push Safety
```bash
# Always use force-with-lease instead of force
git push --force-with-lease origin $BRANCH_NAME

# Check what will be pushed before forcing
git log origin/$BRANCH_NAME..HEAD --oneline
```

## 📊 Quality Checks

### ✅ Pre-Push Validation
```bash
# Run local quality checks
cargo fmt --check
cargo clippy --all-targets --all-features
cargo test
yarn lint
yarn build

# Check commit messages format
cog check --from-latest-tag

# Verify all commits are signed
git log --pretty="format:%h %G? %s" origin/main..HEAD
# Look for 'G' (good signature) or 'U' (unknown signature)
```

### 🔄 Update Reports
After making changes to the release branch, the `update-release-reports.yml` workflow will automatically:
- 🔍 Re-run quality checks
- 🔒 Update security audits
- 🐳 Rebuild Docker images
- 💬 Update PR comments with latest status

## 🎯 Final Steps

1. ✅ Ensure all CI checks pass
2. 🔍 Review PR description and reports
3. 👥 Request code review from team
4. 🚀 Merge PR when approved
5. 🏷️ Create GitHub release from merged commit
6. 📢 Announce release to stakeholders
